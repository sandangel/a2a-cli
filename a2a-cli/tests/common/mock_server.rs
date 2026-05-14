/// Mock A2A server for integration tests.
///
/// Three variants:
/// - [`MockVariant::V1`]         — full A2A v1 server backed by `a2a-server`
/// - [`MockVariant::V03JsonRpc`] — v0.3 JSON-RPC hand-crafted axum handler
/// - [`MockVariant::V03Rest`]    — v0.3 HTTP+JSON REST hand-crafted axum handler
use std::sync::Arc;

use a2a::{
    A2AError, AgentCapabilities, AgentCard, AgentSkill, Artifact, CancelTaskRequest,
    DeleteTaskPushNotificationConfigRequest, GetExtendedAgentCardRequest,
    GetTaskPushNotificationConfigRequest, GetTaskRequest, ListTaskPushNotificationConfigsRequest,
    ListTaskPushNotificationConfigsResponse, ListTasksRequest, ListTasksResponse, Message, Part,
    Role, SendMessageRequest, SendMessageResponse, StreamResponse, SubscribeToTaskRequest, Task,
    TaskPushNotificationConfig, TaskState, TaskStatus, TaskStatusUpdateEvent,
};
use a2a_client::BoxStream;
use a2a_server::{RequestHandler, ServiceParams};
use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::{get, post},
};
use futures::stream;
use serde_json::{Value, json};
use tokio::sync::oneshot;

// ── Variant selector ──────────────────────────────────────────────────

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum MockVariant {
    V1,
    V03JsonRpc,
    V03Rest,
}

// ── MockServer handle ─────────────────────────────────────────────────

/// Handle to a running mock server.  Dropping this shuts it down.
pub struct MockServer {
    pub base_url: String,
    _shutdown_tx: oneshot::Sender<()>,
}

impl MockServer {
    pub async fn start(variant: MockVariant) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind random port");
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");

        let router = build_router(variant, port);

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
                .ok();
        });

        MockServer {
            base_url,
            _shutdown_tx: shutdown_tx,
        }
    }
}

// ── Router factory ────────────────────────────────────────────────────

fn build_router(variant: MockVariant, port: u16) -> Router {
    match variant {
        MockVariant::V1 => build_v1_router(port),
        MockVariant::V03JsonRpc => build_v03_jsonrpc_router(port),
        MockVariant::V03Rest => build_v03_rest_router(port),
    }
}

// ── Shared task fixtures ──────────────────────────────────────────────

pub const MOCK_TASK_ID: &str = "test-task-id-42";
pub const MOCK_CTX_ID: &str = "test-ctx-id-42";
pub const MOCK_CFG_ID: &str = "test-config-id-1";
pub const MOCK_FOLLOW_UP_TEXT: &str = "Follow up using the returned context.";

fn mock_task() -> Task {
    mock_task_with_text("Mock reply from agent.")
}

fn mock_task_with_text(text: &str) -> Task {
    Task {
        id: MOCK_TASK_ID.to_string(),
        context_id: MOCK_CTX_ID.to_string(),
        status: TaskStatus {
            state: TaskState::Completed,
            message: Some(Message {
                message_id: "msg-1".to_string(),
                context_id: Some(MOCK_CTX_ID.to_string()),
                task_id: Some(MOCK_TASK_ID.to_string()),
                role: Role::Agent,
                parts: vec![Part::text(text.to_string())],
                metadata: None,
                extensions: None,
                reference_task_ids: None,
            }),
            timestamp: None,
        },
        artifacts: Some(vec![Artifact {
            artifact_id: "art-1".to_string(),
            name: None,
            description: None,
            parts: vec![Part::text(text.to_string())],
            metadata: None,
            extensions: None,
        }]),
        history: None,
        metadata: None,
    }
}

fn mock_canceled_task() -> Task {
    Task {
        id: MOCK_TASK_ID.to_string(),
        context_id: MOCK_CTX_ID.to_string(),
        status: TaskStatus {
            state: TaskState::Canceled,
            message: None,
            timestamp: None,
        },
        artifacts: None,
        history: None,
        metadata: None,
    }
}

fn mock_push_config(task_id: &str) -> TaskPushNotificationConfig {
    TaskPushNotificationConfig {
        url: "http://127.0.0.1:19999/callback".to_string(),
        id: Some(MOCK_CFG_ID.to_string()),
        task_id: task_id.to_string(),
        token: None,
        authentication: None,
        tenant: None,
    }
}

fn mock_status_stream_event() -> StreamResponse {
    StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
        task_id: MOCK_TASK_ID.to_string(),
        context_id: MOCK_CTX_ID.to_string(),
        status: mock_task().status,
        metadata: None,
    })
}

fn mock_follow_up_task() -> Task {
    mock_task_with_text("Follow-up accepted for exact contextId.")
}

fn checked_follow_up_task(context_id: Option<&str>) -> Result<Task, A2AError> {
    if context_id == Some(MOCK_CTX_ID) {
        return Ok(mock_follow_up_task());
    }
    Err(A2AError::invalid_params(format!(
        "follow-up requires contextId {MOCK_CTX_ID}"
    )))
}

// ── V1 router (backed by a2a-server) ─────────────────────────────────

fn build_v1_router(port: u16) -> Router {
    let handler: Arc<MockV1Handler> = Arc::new(MockV1Handler);
    Router::new()
        .route(
            "/.well-known/agent-card.json",
            get(move || {
                let card = v1_card_json(port);
                async move { Json(card) }
            }),
        )
        .nest("/rpc", a2a_server::jsonrpc::jsonrpc_router(handler))
}

fn v1_card_json(port: u16) -> Value {
    json!({
        "name": "mock-rover",
        "description": "Mock A2A v1 agent for testing all documented commands.",
        "version": "1.0.0",
        "supportedInterfaces": [
            {
                "url": format!("http://127.0.0.1:{port}/rpc"),
                "protocolBinding": "JSONRPC",
                "protocolVersion": "1.0"
            }
        ],
        "capabilities": {
            "streaming": true,
            "pushNotifications": true,
            "extendedAgentCard": true
        },
        "defaultInputModes":  ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": [
            {
                "id": "search-docs",
                "name": "Search Documentation",
                "description": "Search internal documentation and answer questions.",
                "tags": ["search", "docs"],
                "examples": ["What is the A2A protocol?", "How do I authenticate?"],
                "inputModes":  ["text/plain"],
                "outputModes": ["text/plain"]
            }
        ]
    })
}

// ── V1 RequestHandler (full canned responses) ─────────────────────────

struct MockV1Handler;

#[async_trait]
impl RequestHandler for MockV1Handler {
    async fn send_message(
        &self,
        _params: &ServiceParams,
        req: SendMessageRequest,
    ) -> Result<SendMessageResponse, A2AError> {
        if req.message.text() == Some(MOCK_FOLLOW_UP_TEXT) {
            return Ok(SendMessageResponse::Task(checked_follow_up_task(
                req.message.context_id.as_deref(),
            )?));
        }
        Ok(SendMessageResponse::Task(mock_task()))
    }

    async fn send_streaming_message(
        &self,
        _params: &ServiceParams,
        _req: SendMessageRequest,
    ) -> Result<BoxStream<'static, Result<StreamResponse, A2AError>>, A2AError> {
        Ok(Box::pin(stream::once(async {
            Ok(mock_status_stream_event())
        })))
    }

    async fn get_task(
        &self,
        _params: &ServiceParams,
        _req: GetTaskRequest,
    ) -> Result<Task, A2AError> {
        Ok(mock_task())
    }

    async fn list_tasks(
        &self,
        _params: &ServiceParams,
        _req: ListTasksRequest,
    ) -> Result<ListTasksResponse, A2AError> {
        Ok(ListTasksResponse {
            tasks: vec![mock_task()],
            next_page_token: String::new(),
            page_size: 0,
            total_size: 0,
        })
    }

    async fn cancel_task(
        &self,
        _params: &ServiceParams,
        _req: CancelTaskRequest,
    ) -> Result<Task, A2AError> {
        Ok(mock_canceled_task())
    }

    async fn subscribe_to_task(
        &self,
        _params: &ServiceParams,
        _req: SubscribeToTaskRequest,
    ) -> Result<BoxStream<'static, Result<StreamResponse, A2AError>>, A2AError> {
        Ok(Box::pin(stream::once(async {
            Ok(mock_status_stream_event())
        })))
    }

    async fn create_push_config(
        &self,
        _params: &ServiceParams,
        req: TaskPushNotificationConfig,
    ) -> Result<TaskPushNotificationConfig, A2AError> {
        Ok(TaskPushNotificationConfig {
            id: req.id.or_else(|| Some(MOCK_CFG_ID.to_string())),
            ..req
        })
    }

    async fn get_push_config(
        &self,
        _params: &ServiceParams,
        req: GetTaskPushNotificationConfigRequest,
    ) -> Result<TaskPushNotificationConfig, A2AError> {
        Ok(mock_push_config(&req.task_id))
    }

    async fn list_push_configs(
        &self,
        _params: &ServiceParams,
        req: ListTaskPushNotificationConfigsRequest,
    ) -> Result<ListTaskPushNotificationConfigsResponse, A2AError> {
        Ok(ListTaskPushNotificationConfigsResponse {
            configs: vec![mock_push_config(&req.task_id)],
            next_page_token: None,
        })
    }

    async fn delete_push_config(
        &self,
        _params: &ServiceParams,
        _req: DeleteTaskPushNotificationConfigRequest,
    ) -> Result<(), A2AError> {
        Ok(())
    }

    async fn get_extended_agent_card(
        &self,
        _params: &ServiceParams,
        _req: GetExtendedAgentCardRequest,
    ) -> Result<AgentCard, A2AError> {
        Ok(AgentCard {
            name: "mock-rover (extended)".to_string(),
            description: "Extended mock card with full skill details.".to_string(),
            version: "1.0.0".to_string(),
            capabilities: AgentCapabilities {
                streaming: Some(true),
                push_notifications: Some(true),
                extended_agent_card: Some(true),
                extensions: None,
            },
            skills: vec![AgentSkill {
                id: "search-docs".to_string(),
                name: "Search Documentation".to_string(),
                description: "Extended: full-text search across all internal docs.".to_string(),
                tags: vec!["search".to_string(), "docs".to_string()],
                examples: Some(vec!["What is the A2A protocol?".to_string()]),
                input_modes: Some(vec!["text/plain".to_string()]),
                output_modes: Some(vec!["text/plain".to_string()]),
                security_requirements: None,
            }],
            default_input_modes: vec!["text/plain".to_string()],
            default_output_modes: vec!["text/plain".to_string()],
            supported_interfaces: vec![],
            security_schemes: None,
            security_requirements: None,
            provider: None,
            documentation_url: None,
            icon_url: None,
            signatures: None,
        })
    }
}

// ── V0.3 shared task JSON ─────────────────────────────────────────────

fn v03_task_json() -> Value {
    v03_task_json_with_text("Mock reply from agent.")
}

fn v03_task_json_with_text(text: &str) -> Value {
    json!({
        "id": MOCK_TASK_ID,
        "contextId": MOCK_CTX_ID,
        "status": { "state": "completed" },
        "artifacts": [
            {
                "artifactId": "art-1",
                "parts": [{ "kind": "text", "text": text }]
            }
        ]
    })
}

fn v03_follow_up_task_json() -> Value {
    v03_task_json_with_text("Follow-up accepted for exact contextId.")
}

fn v03_canceled_task_json() -> Value {
    json!({
        "id": MOCK_TASK_ID,
        "contextId": MOCK_CTX_ID,
        "status": { "state": "canceled" }
    })
}

fn v03_push_config_json() -> Value {
    json!({
        "taskId": MOCK_TASK_ID,
        "pushNotificationConfig": {
            "url": "http://127.0.0.1:19999/callback",
            "id": MOCK_CFG_ID
        }
    })
}

/// One-shot SSE response: sends a single status-update event and closes.
fn single_sse_response(
    task: Value,
) -> Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let data = serde_json::to_string(&task).unwrap_or_default();
    Sse::new(stream::once(async move {
        Ok::<Event, std::convert::Infallible>(Event::default().data(data))
    }))
}

fn jsonrpc_ok(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn jsonrpc_error(id: &Value, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32602,
            "message": message.into(),
        }
    })
}

fn message_text(body: &Value) -> Option<&str> {
    body.pointer("/message/parts/0/text")
        .and_then(|v| v.as_str())
}

fn message_context_id<'a>(body: &'a Value, key: &str) -> Option<&'a str> {
    body.get("message")?.get(key)?.as_str()
}

fn checked_v03_follow_up_task(body: &Value, context_key: &str) -> Result<Value, String> {
    if message_context_id(body, context_key) == Some(MOCK_CTX_ID) {
        return Ok(v03_follow_up_task_json());
    }
    Err(format!("follow-up requires contextId {MOCK_CTX_ID}"))
}

// ── V0.3 JSON-RPC router ──────────────────────────────────────────────

fn build_v03_jsonrpc_router(port: u16) -> Router {
    Router::new()
        .route(
            "/.well-known/agent-card.json",
            get(move || {
                let card = v03_jsonrpc_card_json(port);
                async move { Json(card) }
            }),
        )
        .route("/rpc", post(v03_jsonrpc_handler))
}

fn v03_jsonrpc_card_json(port: u16) -> Value {
    json!({
        "name": "mock-eai",
        "description": "Mock A2A v0.3 JSON-RPC agent",
        "version": "1.0.0",
        "protocolVersion": "0.3.0",
        "url": format!("http://127.0.0.1:{port}/rpc"),
        "preferredTransport": "JSONRPC",
        "capabilities": { "streaming": true },
        "defaultInputModes":  ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": [
            {
                "id": "general",
                "name": "General Assistant",
                "description": "Answers general questions."
            }
        ]
    })
}

async fn v03_jsonrpc_handler(Json(req): Json<Value>) -> Json<Value> {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = req.get("params").unwrap_or(&Value::Null);

    if matches!(method, "message/send" | "tasks/send")
        && message_text(params) == Some(MOCK_FOLLOW_UP_TEXT)
    {
        return match checked_v03_follow_up_task(params, "contextId") {
            Ok(result) => Json(jsonrpc_ok(&id, result)),
            Err(message) => Json(jsonrpc_error(&id, message)),
        };
    }

    let result = match method {
        "message/send" | "tasks/send" => v03_task_json(),
        "tasks/get" => v03_task_json(),
        "tasks/list" => json!({ "tasks": [v03_task_json()] }),
        "tasks/cancel" => v03_canceled_task_json(),
        _ => json!({ "error": format!("unknown method: {method}") }),
    };

    Json(jsonrpc_ok(&id, result))
}

// ── V0.3 REST router ──────────────────────────────────────────────────

fn build_v03_rest_router(port: u16) -> Router {
    Router::new()
        .route(
            "/.well-known/agent-card.json",
            get(move || {
                let card = v03_rest_card_json(port);
                async move { Json(card) }
            }),
        )
        .route("/message:send", post(v03_rest_send))
        .route("/message:stream", post(v03_rest_stream))
        .route("/tasks", get(v03_rest_list_tasks))
        .route(
            "/tasks/{*path}",
            get(v03_rest_tasks_get).post(v03_rest_tasks_post),
        )
}

fn v03_rest_card_json(port: u16) -> Value {
    json!({
        "name": "mock-eai",
        "description": "Mock A2A v0.3 REST agent",
        "version": "1.0.0",
        "protocolVersion": "0.3.0",
        "url": format!("http://127.0.0.1:{port}"),
        "preferredTransport": "HTTP+JSON",
        "capabilities": { "streaming": true, "pushNotifications": false },
        "supportsAuthenticatedExtendedCard": false,
        "defaultInputModes":  ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": [
            {
                "id": "general",
                "name": "General Assistant",
                "description": "Answers general questions."
            }
        ]
    })
}

fn v03_rest_task_snake() -> Value {
    v03_rest_task_snake_with_text("Mock reply from agent.")
}

fn v03_rest_task_snake_with_text(text: &str) -> Value {
    json!({
        "id": MOCK_TASK_ID,
        "context_id": MOCK_CTX_ID,
        "status": { "state": "completed" },
        "artifacts": [
            {
                "artifact_id": "art-1",
                "parts": [{ "kind": "text", "text": text }]
            }
        ]
    })
}

fn v03_rest_follow_up_task_snake() -> Value {
    v03_rest_task_snake_with_text("Follow-up accepted for exact contextId.")
}

fn v03_rest_canceled_task_snake() -> Value {
    json!({
        "id": MOCK_TASK_ID,
        "context_id": MOCK_CTX_ID,
        "status": { "state": "canceled" }
    })
}

async fn v03_rest_send(body: Option<Json<Value>>) -> Response {
    if let Some(Json(body)) = body
        && message_text(&body) == Some(MOCK_FOLLOW_UP_TEXT)
    {
        return match checked_v03_follow_up_task(&body, "context_id") {
            Ok(_) => Json(v03_rest_follow_up_task_snake()).into_response(),
            Err(message) => {
                (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
            }
        };
    }
    Json(v03_rest_task_snake()).into_response()
}

async fn v03_rest_stream(
    _body: Option<Json<Value>>,
) -> Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>> {
    single_sse_response(v03_task_json())
}

async fn v03_rest_list_tasks() -> Json<Value> {
    Json(json!({ "tasks": [v03_rest_task_snake()] }))
}

async fn v03_rest_tasks_get(Path(_path): Path<String>) -> Json<Value> {
    Json(v03_rest_task_snake())
}

/// Routes POST /tasks/{*path} — cancel, subscribe, or push-config operations.
async fn v03_rest_tasks_post(Path(path): Path<String>, body: Option<Json<Value>>) -> Response {
    if path.ends_with(":cancel") {
        Json(v03_rest_canceled_task_snake()).into_response()
    } else if path.ends_with(":subscribe") {
        single_sse_response(v03_task_json()).into_response()
    } else if path.contains("pushNotificationConfigs") {
        // push-config create: POST /tasks/{id}/pushNotificationConfigs
        let cfg = if let Some(Json(b)) = body {
            let url = b
                .pointer("/pushNotificationConfig/url")
                .and_then(|v| v.as_str())
                .unwrap_or("http://127.0.0.1:19999/callback")
                .to_string();
            json!({ "taskId": MOCK_TASK_ID, "pushNotificationConfig": { "url": url, "id": MOCK_CFG_ID } })
        } else {
            v03_push_config_json()
        };
        Json(cfg).into_response()
    } else {
        Json(json!({ "error": format!("unknown path: /tasks/{path}") })).into_response()
    }
}

// ── Push-config GET endpoints for v0.3 REST ───────────────────────────
// These are handled via the wildcard above for POST, but GET requires the
// list route to be separate from the per-config route.  Since all /tasks/*
// GETs go through v03_rest_tasks_get, they return the task JSON, which is
// close enough for testing purposes.  The v1 mock handles push-config fully.
