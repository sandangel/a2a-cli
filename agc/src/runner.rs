//! run_to_value — mirrors a2acli's run() but returns serde_json::Value
//! instead of printing, enabling --fields filtering.

use std::sync::Arc;

use a2a::{
    AgentCard, CancelTaskRequest, CreateTaskPushNotificationConfigRequest,
    DeleteTaskPushNotificationConfigRequest, GetExtendedAgentCardRequest,
    GetTaskPushNotificationConfigRequest, GetTaskRequest, ListTaskPushNotificationConfigsRequest,
    ListTasksRequest, Message, Part, PushNotificationConfig, AuthenticationInfo,
    Role, SendMessageConfiguration, SendMessageRequest, SubscribeToTaskRequest,
    TaskState,
};
use a2a_client::{A2AClient, A2AClientFactory, agent_card::AgentCardResolver, auth::AuthInterceptor};
use a2acli::{Binding, MessageCommand, PushConfigCommand, TaskIdCommand, TaskLookupCommand, ListTasksCommand};
use crate::cli::Command;
use futures::StreamExt;
use serde_json::Value;

use crate::error::{AgcError, Result};

// ── Public entry points ───────────────────────────────────────────────

/// Detect whether a raw card JSON is A2A v0.3 (protocolVersion starts with "0.").
fn is_v03(raw: &Value) -> bool {
    raw.get("protocolVersion")
        .and_then(|v| v.as_str())
        .map(|v| v.starts_with("0."))
        .unwrap_or(false)
}

/// Run a non-streaming command and return the result as a JSON Value.
/// Streaming commands (Stream, Subscribe) must use run_streaming().
pub async fn run_to_value(
    command: &Command,
    base_url: &str,
    bearer: Option<&str>,
    binding: Option<Binding>,
    tenant: Option<&str>,
) -> Result<Value> {
    // Detect v0.3 and dispatch to compat path.
    let raw = fetch_card_raw(base_url, bearer).await?;
    if is_v03(&raw) {
        let rpc_url = raw.get("url").and_then(|u| u.as_str())
            .unwrap_or(base_url);
        return run_v03_to_value(command, rpc_url, bearer, tenant).await;
    }

    match command {
        Command::Card => {
            let card = fetch_card(base_url, bearer).await?;
            Ok(serde_json::to_value(&card)?)
        }
        Command::ExtendedCard => {
            let client = build_client(base_url, bearer, binding).await?;
            let result = client
                .get_extended_agent_card(&GetExtendedAgentCardRequest {
                    tenant: tenant.map(str::to_string),
                })
                .await;
            let card = finish(client, result).await?;
            Ok(serde_json::to_value(&card)?)
        }
        Command::Send(cmd) => {
            let req = build_send_request(cmd, tenant);
            let client = build_client(base_url, bearer, binding).await?;
            let result = client.send_message(&req).await;
            let resp = finish(client, result).await?;
            Ok(serde_json::to_value(&resp)?)
        }
        Command::GetTask(cmd) => {
            let client = build_client(base_url, bearer, binding).await?;
            let result = client
                .get_task(&GetTaskRequest {
                    id: cmd.id.clone(),
                    history_length: cmd.history_length,
                    tenant: tenant.map(str::to_string),
                })
                .await;
            let task = finish(client, result).await?;
            Ok(serde_json::to_value(&task)?)
        }
        Command::ListTasks(cmd) => {
            let client = build_client(base_url, bearer, binding).await?;
            let result = client
                .list_tasks(&ListTasksRequest {
                    context_id: cmd.context_id.clone(),
                    status: cmd.status.map(|s| TaskState::from(s)),
                    page_size: cmd.page_size,
                    page_token: cmd.page_token.clone(),
                    history_length: cmd.history_length,
                    include_artifacts: cmd.include_artifacts.then_some(true),
                    status_timestamp_after: None,
                    tenant: tenant.map(str::to_string),
                })
                .await;
            let resp = finish(client, result).await?;
            Ok(serde_json::to_value(&resp)?)
        }
        Command::CancelTask(cmd) => {
            let client = build_client(base_url, bearer, binding).await?;
            let result = client
                .cancel_task(&a2a::CancelTaskRequest {
                    id: cmd.id.clone(),
                    metadata: None,
                    tenant: tenant.map(str::to_string),
                })
                .await;
            let task = finish(client, result).await?;
            Ok(serde_json::to_value(&task)?)
        }
        Command::PushConfig { command } => run_push_to_value(command, base_url, bearer, binding, tenant).await,
        // Streaming commands handled separately.
        Command::Agent { .. } | Command::Auth { .. } | Command::Config { .. }
        | Command::Schema { .. } | Command::GenerateSkills(_) => {
            unreachable!("management commands handled before runner")
        }
        Command::Stream(_) | Command::Subscribe(_) => Err(AgcError::InvalidInput(
            "use run_streaming() for streaming commands".to_string(),
        )),
    }
}

/// Run a streaming command, calling `on_event` for each Value received.
pub async fn run_streaming(
    command: &Command,
    base_url: &str,
    bearer: Option<&str>,
    binding: Option<Binding>,
    tenant: Option<&str>,
    on_event: impl FnMut(Value) -> Result<()>,
) -> Result<()> {
    let raw = fetch_card_raw(base_url, bearer).await?;
    if is_v03(&raw) {
        let rpc_url = raw.get("url").and_then(|u| u.as_str()).unwrap_or(base_url);
        return run_v03_streaming(command, rpc_url, bearer, tenant, on_event).await;
    }

    let mut on_event = on_event;
    match command {
        Command::Stream(cmd) => {
            let req = build_send_request(cmd, tenant);
            let client = build_client(base_url, bearer, binding).await?;
            let mut stream = client
                .send_streaming_message(&req)
                .await
                .map_err(AgcError::A2A)?;
            while let Some(event) = stream.next().await {
                match event {
                    Ok(e) => on_event(serde_json::to_value(&e)?)?,
                    Err(e) => eprintln!("stream error: {e}"),
                }
            }
            let _ = client.destroy().await;
        }
        Command::Subscribe(cmd) => {
            let client = build_client(base_url, bearer, binding).await?;
            let mut stream = client
                .subscribe_to_task(&SubscribeToTaskRequest {
                    id: cmd.id.clone(),
                    tenant: tenant.map(str::to_string),
                })
                .await
                .map_err(AgcError::A2A)?;
            while let Some(event) = stream.next().await {
                match event {
                    Ok(e) => on_event(serde_json::to_value(&e)?)?,
                    Err(e) => eprintln!("stream error: {e}"),
                }
            }
            let _ = client.destroy().await;
        }
        _ => return Err(AgcError::InvalidInput("not a streaming command".to_string())),
    }
    Ok(())
}

pub fn is_streaming(command: &Command) -> bool {
    matches!(command, Command::Stream(_) | Command::Subscribe(_))
}

// ── Push config ───────────────────────────────────────────────────────

async fn run_push_to_value(
    command: &PushConfigCommand,
    base_url: &str,
    bearer: Option<&str>,
    binding: Option<Binding>,
    tenant: Option<&str>,
) -> Result<Value> {
    let client = build_client(base_url, bearer, binding).await?;
    let tenant = tenant.map(str::to_string);

    match command {
        PushConfigCommand::Create(cmd) => {
            if cmd.auth_credentials.is_some() && cmd.auth_scheme.is_none() {
                return Err(AgcError::InvalidInput(
                    "--auth-credentials requires --auth-scheme".to_string(),
                ));
            }
            let config = PushNotificationConfig {
                url: cmd.url.clone(),
                id: cmd.config_id.clone(),
                token: cmd.token.clone(),
                authentication: cmd.auth_scheme.clone().map(|scheme| AuthenticationInfo {
                    scheme,
                    credentials: cmd.auth_credentials.clone(),
                }),
            };
            let result = client
                .create_push_config(&CreateTaskPushNotificationConfigRequest {
                    task_id: cmd.task_id.clone(),
                    config,
                    tenant,
                })
                .await;
            let resp = finish(client, result).await?;
            Ok(serde_json::to_value(&resp)?)
        }
        PushConfigCommand::Get(cmd) => {
            let result = client
                .get_push_config(&GetTaskPushNotificationConfigRequest {
                    task_id: cmd.task_id.clone(),
                    id: cmd.id.clone(),
                    tenant,
                })
                .await;
            let resp = finish(client, result).await?;
            Ok(serde_json::to_value(&resp)?)
        }
        PushConfigCommand::List(cmd) => {
            let result = client
                .list_push_configs(&ListTaskPushNotificationConfigsRequest {
                    task_id: cmd.task_id.clone(),
                    page_size: cmd.page_size,
                    page_token: cmd.page_token.clone(),
                    tenant,
                })
                .await;
            let resp = finish(client, result).await?;
            Ok(serde_json::to_value(&resp)?)
        }
        PushConfigCommand::Delete(cmd) => {
            let result = client
                .delete_push_config(&DeleteTaskPushNotificationConfigRequest {
                    task_id: cmd.task_id.clone(),
                    id: cmd.id.clone(),
                    tenant,
                })
                .await;
            finish(client, result).await?;
            Ok(serde_json::json!({ "deleted": true, "task_id": cmd.task_id, "id": cmd.id }))
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────

pub async fn fetch_card(base_url: &str, bearer: Option<&str>) -> Result<AgentCard> {
    let http = build_http_client(bearer)?;
    // Try v1 parse first; fall back to v0.3 normalization.
    match AgentCardResolver::new(Some(http)).resolve(base_url).await {
        Ok(card) => Ok(card),
        Err(_) => {
            let raw = fetch_card_raw(base_url, bearer).await?;
            normalize_v03_card(&raw)
        }
    }
}

/// Normalize an A2A v0.3 card JSON into a v1 AgentCard.
fn normalize_v03_card(raw: &serde_json::Value) -> Result<AgentCard> {
    use serde_json::{json, Map};

    let mut v1 = raw.as_object().cloned().unwrap_or_default();

    // v0.3 has a top-level `url` pointing to the RPC endpoint.
    // Convert to `supportedInterfaces` array.
    if !v1.contains_key("supportedInterfaces") {
        if let Some(url) = v1.get("url").and_then(|u| u.as_str()) {
            let binding = v1.get("preferredTransport")
                .and_then(|t| t.as_str())
                .unwrap_or("JSONRPC");
            v1.insert("supportedInterfaces".into(), json!([{
                "url": url,
                "protocolBinding": binding,
                "protocolVersion": "1.0"
            }]));
        }
    }

    // Normalize security schemes: v0.3 uses `type: "oauth2"`, v1 uses `oauth2SecurityScheme` key.
    if let Some(schemes) = v1.get("securitySchemes").and_then(|s| s.as_object()).cloned() {
        let mut normalized: Map<String, serde_json::Value> = Map::new();
        for (name, scheme) in &schemes {
            if scheme.get("oauth2SecurityScheme").is_some() {
                normalized.insert(name.clone(), scheme.clone()); // already v1
            } else if scheme.get("type").and_then(|t| t.as_str()) == Some("oauth2") {
                // Wrap flows under oauth2SecurityScheme key.
                let mut inner = scheme.as_object().cloned().unwrap_or_default();
                inner.remove("type");
                normalized.insert(name.clone(), json!({ "oauth2SecurityScheme": inner }));
            } else {
                normalized.insert(name.clone(), scheme.clone());
            }
        }
        v1.insert("securitySchemes".into(), serde_json::Value::Object(normalized));
    }

    serde_json::from_value(serde_json::Value::Object(v1))
        .map_err(|e| AgcError::A2A(a2a::A2AError::internal(format!("card parse: {e}"))))
}

/// Fetch the agent card as raw JSON — handles any protocol version without
/// failing on schema differences (e.g. v0.3 vs v1).
pub async fn fetch_card_raw(base_url: &str, bearer: Option<&str>) -> Result<serde_json::Value> {
    let http = build_http_client(bearer)?;
    let url = format!("{}/.well-known/agent-card.json", base_url.trim_end_matches('/'));
    let resp = http.get(&url).send().await.map_err(AgcError::Http)?;
    if !resp.status().is_success() {
        return Err(AgcError::A2A(a2a::A2AError::internal(
            format!("agent card fetch returned HTTP {}", resp.status())
        )));
    }
    resp.json().await.map_err(AgcError::Http)
}

fn build_http_client(bearer: Option<&str>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder();
    if let Some(token) = bearer {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {token}").parse().map_err(|e| AgcError::Auth(format!("{e}")))?,
        );
        builder = builder.default_headers(headers);
    }
    builder.build().map_err(AgcError::Http)
}

async fn build_client(base_url: &str, bearer: Option<&str>, binding: Option<Binding>) -> Result<A2AClient> {
    let card = fetch_card(base_url, bearer).await?;
    let mut builder = A2AClientFactory::builder();
    if let Some(b) = binding {
        let proto = match b { Binding::Jsonrpc => "JSONRPC", Binding::HttpJson => "HTTP+JSON" };
        builder = builder.preferred_bindings(vec![proto.to_string()]);
    }

    if let Some(token) = bearer {
        builder = builder.with_interceptor(Arc::new(AuthInterceptor::bearer(token)));
    }
    builder.build().create_from_card(&card).await.map_err(AgcError::A2A)
}

async fn finish<T>(client: A2AClient, result: std::result::Result<T, a2a::A2AError>) -> Result<T> {
    let _ = client.destroy().await;
    result.map_err(AgcError::A2A)
}

// ── A2A v0.3 compat runner ────────────────────────────────────────────
//
// v0.3 uses different JSONRPC method names and request/response formats.
// Method mapping:  message/send, message/stream, tasks/get, tasks/cancel, tasks/resubscribe

async fn run_v03_to_value(
    command: &Command,
    rpc_url: &str,
    bearer: Option<&str>,
    tenant: Option<&str>,
) -> Result<Value> {
    match command {
        Command::Card | Command::ExtendedCard => fetch_card_raw(rpc_url, bearer).await,
        Command::Send(cmd) => {
            jsonrpc_call(rpc_url, bearer, "message/send",
                v03_send_params(&cmd.text, cmd.context_id.as_deref(), cmd.task_id.as_deref(),
                    cmd.history_length, cmd.return_immediately, tenant)).await
        }
        Command::GetTask(cmd) => {
            jsonrpc_call(rpc_url, bearer, "tasks/get",
                serde_json::json!({ "id": cmd.id, "historyLength": cmd.history_length })).await
        }
        Command::ListTasks(cmd) => {
            jsonrpc_call(rpc_url, bearer, "tasks/list",
                serde_json::json!({ "contextId": cmd.context_id, "pageSize": cmd.page_size,
                    "pageToken": cmd.page_token })).await
        }
        Command::CancelTask(cmd) => {
            jsonrpc_call(rpc_url, bearer, "tasks/cancel",
                serde_json::json!({ "id": cmd.id })).await
        }
        Command::Stream(_) | Command::Subscribe(_) => Err(AgcError::InvalidInput(
            "use run_streaming() for streaming commands".to_string())),
        Command::PushConfig { .. } => Err(AgcError::InvalidInput(
            "push-config not supported for v0.3 agents".to_string())),
        Command::Agent { .. } | Command::Auth { .. } | Command::Config { .. }
        | Command::Schema { .. } | Command::GenerateSkills(_) => {
            unreachable!("management commands handled before runner")
        }
    }
}

pub async fn run_v03_streaming(
    command: &Command,
    rpc_url: &str,
    bearer: Option<&str>,
    tenant: Option<&str>,
    mut on_event: impl FnMut(Value) -> Result<()>,
) -> Result<()> {
    use tokio::io::AsyncBufReadExt;

    let (method, params) = match command {
        Command::Stream(cmd) => ("message/stream",
            v03_send_params(&cmd.text, cmd.context_id.as_deref(), cmd.task_id.as_deref(),
                cmd.history_length, false, tenant)),
        Command::Subscribe(cmd) => ("tasks/resubscribe",
            serde_json::json!({ "id": cmd.id })),
        _ => return Err(AgcError::InvalidInput("not a streaming command".to_string())),
    };

    let body = serde_json::json!({
        "jsonrpc": "2.0", "id": uuid::Uuid::new_v4().to_string(),
        "method": method, "params": params,
    });

    let resp = build_http_client(bearer)?
        .post(rpc_url).header("Content-Type", "application/json").json(&body)
        .send().await.map_err(AgcError::Http)?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        return Err(AgcError::A2A(a2a::A2AError::internal(format!("HTTP {status}: {text}"))));
    }

    let stream = resp.bytes_stream().map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));
    let mut reader = tokio::io::BufReader::new(tokio_util::io::StreamReader::new(stream));
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await.map_err(AgcError::Io)? == 0 { break; }
        let trimmed = line.trim_start_matches("data:").trim();
        if trimmed.is_empty() || trimmed == "[DONE]" { continue; }
        if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
            on_event(v)?;
        }
    }
    Ok(())
}

async fn jsonrpc_call(rpc_url: &str, bearer: Option<&str>, method: &str, params: Value) -> Result<Value> {
    let body = serde_json::json!({
        "jsonrpc": "2.0", "id": uuid::Uuid::new_v4().to_string(),
        "method": method, "params": params,
    });
    let json: Value = build_http_client(bearer)?
        .post(rpc_url).header("Content-Type", "application/json").json(&body)
        .send().await.map_err(AgcError::Http)?
        .json().await.map_err(AgcError::Http)?;

    if let Some(err) = json.get("error") {
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
        return Err(AgcError::A2A(a2a::A2AError::internal(format!("jsonrpc error: {msg}"))));
    }
    Ok(json.get("result").cloned().unwrap_or(json))
}

fn v03_send_params(text: &str, context_id: Option<&str>, task_id: Option<&str>,
    history_length: Option<i32>, return_immediately: bool, tenant: Option<&str>) -> Value {
    serde_json::json!({
        "message": {
            "role": "user",
            "parts": [{ "type": "text", "text": text }],
            "messageId": uuid::Uuid::new_v4().to_string(),
            "contextId": context_id,
            "taskId": task_id,
        },
        "config": {
            "blocking": !return_immediately,
            "historyLength": history_length,
            "acceptedOutputModes": ["text/plain", "text/markdown"],
        },
        "metadata": { "tenant": tenant },
    })
}

// ── Build send request (v1) ───────────────────────────────────────────

fn build_send_request(cmd: &MessageCommand, tenant: Option<&str>) -> SendMessageRequest {
    let mut msg = Message::new(Role::User, vec![Part::text(cmd.text.clone())]);
    msg.context_id = cmd.context_id.clone();
    msg.task_id = cmd.task_id.clone();

    let configuration = if cmd.history_length.is_some()
        || !cmd.accepted_output_modes.is_empty()
        || cmd.return_immediately
    {
        Some(SendMessageConfiguration {
            accepted_output_modes: (!cmd.accepted_output_modes.is_empty())
                .then_some(cmd.accepted_output_modes.clone()),
            history_length: cmd.history_length,
            return_immediately: cmd.return_immediately.then_some(true),
            push_notification_config: None,
        })
    } else {
        None
    };

    SendMessageRequest {
        message: msg,
        configuration,
        metadata: None,
        tenant: tenant.map(str::to_string),
    }
}
