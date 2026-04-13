/// Mock A2A server for integration tests.
///
/// Three variants:
/// - [`MockVariant::V1`]       — full A2A v1 server backed by `a2a-server`
/// - [`MockVariant::V03JsonRpc`] — v0.3 JSON-RPC hand-crafted axum handler
/// - [`MockVariant::V03Rest`]  — v0.3 HTTP+JSON REST hand-crafted axum handler
use std::sync::Arc;

use a2a::{
    A2AError, AgentCard, Artifact, CancelTaskRequest, CreateTaskPushNotificationConfigRequest,
    DeleteTaskPushNotificationConfigRequest, GetExtendedAgentCardRequest,
    GetTaskPushNotificationConfigRequest, GetTaskRequest, ListTaskPushNotificationConfigsRequest,
    ListTaskPushNotificationConfigsResponse, ListTasksRequest, ListTasksResponse, Message, Part,
    Role, SendMessageRequest, SendMessageResponse, SubscribeToTaskRequest, Task,
    TaskPushNotificationConfig, TaskState, TaskStatus,
};
use a2a_client::BoxStream;
use a2a_server::{RequestHandler, ServiceParams};
use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::Path,
    routing::{get, post},
};
use serde_json::{Value, json};
use tokio::sync::oneshot;

// ── Variant selector ──────────────────────────────────────────────────

#[derive(Clone, Copy)]
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
        "description": "Mock A2A v1 agent",
        "version": "1.0.0",
        "supportedInterfaces": [
            {
                "url": format!("http://127.0.0.1:{port}/rpc"),
                "protocolBinding": "JSONRPC",
                "protocolVersion": "1.0"
            }
        ],
        "capabilities": {},
        "defaultInputModes":  ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": []
    })
}

// ── V1 RequestHandler (canned responses) ─────────────────────────────

struct MockV1Handler;

fn mock_task() -> Task {
    Task {
        id: "test-task-id-42".to_string(),
        context_id: "test-ctx-id-42".to_string(),
        status: TaskStatus {
            state: TaskState::Completed,
            message: Some(Message {
                message_id: "msg-1".to_string(),
                context_id: Some("test-ctx-id-42".to_string()),
                task_id: Some("test-task-id-42".to_string()),
                role: Role::Agent,
                parts: vec![Part::text("Mock reply from agent.")],
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
            parts: vec![Part::text("Mock reply from agent.")],
            metadata: None,
            extensions: None,
        }]),
        history: None,
        metadata: None,
    }
}

fn mock_canceled_task() -> Task {
    Task {
        id: "test-task-id-42".to_string(),
        context_id: "test-ctx-id-42".to_string(),
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

#[async_trait]
impl RequestHandler for MockV1Handler {
    async fn send_message(
        &self,
        _params: &ServiceParams,
        _req: SendMessageRequest,
    ) -> Result<SendMessageResponse, A2AError> {
        Ok(SendMessageResponse::Task(mock_task()))
    }

    async fn send_streaming_message(
        &self,
        _params: &ServiceParams,
        _req: SendMessageRequest,
    ) -> Result<BoxStream<'static, Result<a2a::StreamResponse, A2AError>>, A2AError> {
        Err(A2AError::unsupported_operation(
            "streaming not used in tests",
        ))
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
    ) -> Result<BoxStream<'static, Result<a2a::StreamResponse, A2AError>>, A2AError> {
        Err(A2AError::unsupported_operation(
            "subscribe not used in tests",
        ))
    }

    async fn create_push_config(
        &self,
        _params: &ServiceParams,
        _req: CreateTaskPushNotificationConfigRequest,
    ) -> Result<TaskPushNotificationConfig, A2AError> {
        Err(A2AError::unsupported_operation("push not used in tests"))
    }

    async fn get_push_config(
        &self,
        _params: &ServiceParams,
        _req: GetTaskPushNotificationConfigRequest,
    ) -> Result<TaskPushNotificationConfig, A2AError> {
        Err(A2AError::unsupported_operation("push not used in tests"))
    }

    async fn list_push_configs(
        &self,
        _params: &ServiceParams,
        _req: ListTaskPushNotificationConfigsRequest,
    ) -> Result<ListTaskPushNotificationConfigsResponse, A2AError> {
        Err(A2AError::unsupported_operation("push not used in tests"))
    }

    async fn delete_push_config(
        &self,
        _params: &ServiceParams,
        _req: DeleteTaskPushNotificationConfigRequest,
    ) -> Result<(), A2AError> {
        Err(A2AError::unsupported_operation("push not used in tests"))
    }

    async fn get_extended_agent_card(
        &self,
        _params: &ServiceParams,
        _req: GetExtendedAgentCardRequest,
    ) -> Result<AgentCard, A2AError> {
        Err(A2AError::unsupported_operation(
            "extended card not used in tests",
        ))
    }
}

// ── V0.3 shared task JSON ─────────────────────────────────────────────

/// Canned v0.3 task response — returned directly as JSON-RPC result.
fn v03_task_json() -> Value {
    json!({
        "id": "test-task-id-42",
        "contextId": "test-ctx-id-42",
        "status": { "state": "completed" },
        "artifacts": [
            {
                "artifactId": "art-1",
                "parts": [{ "kind": "text", "text": "Mock reply from agent." }]
            }
        ]
    })
}

/// Canned v0.3 canceled task response.
fn v03_canceled_task_json() -> Value {
    json!({
        "id": "test-task-id-42",
        "contextId": "test-ctx-id-42",
        "status": { "state": "canceled" }
    })
}

/// Wrap a value in a JSON-RPC 2.0 success envelope.
fn jsonrpc_ok(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
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
        "capabilities": {},
        "defaultInputModes":  ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": []
    })
}

async fn v03_jsonrpc_handler(Json(req): Json<Value>) -> Json<Value> {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");

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
        // Note: REST responses are snake_case; the a2a-compat client converts them
        // to camelCase before returning. See a2a_compat::Client::rest_request.
        .route("/message:send", post(v03_rest_send))
        .route("/tasks", get(v03_rest_list_tasks))
        // Use a wildcard to handle both GET /tasks/{id} and POST /tasks/{id}:cancel.
        // axum 0.8 does not allow both `/tasks/{id}` (GET) and `/tasks/{*path}`
        // (POST) to coexist, so we handle all /tasks/* with method routing on a
        // single wildcard route.
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
        "capabilities": {},
        "defaultInputModes":  ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": []
    })
}

/// v0.3 REST task fixture in snake_case (the client converts to camelCase).
fn v03_rest_task_snake() -> Value {
    json!({
        "id": "test-task-id-42",
        "context_id": "test-ctx-id-42",
        "status": { "state": "completed" },
        "artifacts": [
            {
                "artifact_id": "art-1",
                "parts": [{ "kind": "text", "text": "Mock reply from agent." }]
            }
        ]
    })
}

fn v03_rest_canceled_task_snake() -> Value {
    json!({
        "id": "test-task-id-42",
        "context_id": "test-ctx-id-42",
        "status": { "state": "canceled" }
    })
}

async fn v03_rest_send(_body: Option<Json<Value>>) -> Json<Value> {
    Json(v03_rest_task_snake())
}

async fn v03_rest_list_tasks() -> Json<Value> {
    Json(json!({ "tasks": [v03_rest_task_snake()] }))
}

/// Handles `GET /tasks/{*path}` — any GET under /tasks/ returns the task.
async fn v03_rest_tasks_get(Path(_path): Path<String>) -> Json<Value> {
    Json(v03_rest_task_snake())
}

/// Handles `POST /tasks/{*path}` — dispatches on whether the path ends with `:cancel`.
async fn v03_rest_tasks_post(Path(path): Path<String>) -> Json<Value> {
    if path.ends_with(":cancel") {
        Json(v03_rest_canceled_task_snake())
    } else {
        // Unknown sub-path — return an error object (tests won't hit this).
        Json(json!({ "error": format!("unknown path: /tasks/{path}") }))
    }
}
