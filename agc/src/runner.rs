//! run_to_value — mirrors a2acli's run() but returns serde_json::Value
//! instead of printing, enabling --fields filtering.
//!
//! A2A v0.3 agents are handled transparently: `run_to_value` and
//! `run_streaming` detect the protocol version from the card and delegate
//! v0.3 calls to the [`a2a_compat`] crate.

use std::sync::Arc;

use a2a::{
    AgentCard, CreateTaskPushNotificationConfigRequest,
    DeleteTaskPushNotificationConfigRequest, GetExtendedAgentCardRequest,
    GetTaskPushNotificationConfigRequest, GetTaskRequest, ListTaskPushNotificationConfigsRequest,
    ListTasksRequest, Message, Part, PushNotificationConfig, AuthenticationInfo,
    Role, SendMessageConfiguration, SendMessageRequest, SubscribeToTaskRequest,
    TaskState,
};
use a2a_client::{A2AClient, A2AClientFactory, auth::AuthInterceptor};
use a2a_compat::MessageParams;
use a2acli::{Binding, MessageCommand, PushConfigCommand};
use crate::cli::Command;
use futures::StreamExt;
use serde_json::Value;

use crate::error::{AgcError, Result};

// ── Public entry points ───────────────────────────────────────────────

/// Run a non-streaming command and return the result as a JSON Value.
/// Streaming commands (Stream, Subscribe) must use [`run_streaming`].
pub async fn run_to_value(
    command: &Command,
    base_url: &str,
    bearer: Option<&str>,
    binding: Option<Binding>,
    tenant: Option<&str>,
) -> Result<Value> {
    // Fetch once — reuse for both version detection and client construction.
    let raw = fetch_card_raw(base_url, bearer).await?;

    if a2a_compat::is_v03(&raw) {
        // Card commands: return the already-fetched raw card (avoids a second fetch).
        if matches!(command, Command::Card | Command::ExtendedCard) {
            return Ok(raw);
        }
        let url = a2a_compat::rpc_url_from_card(&raw, base_url);
        let client = a2a_compat::Client::new(url, bearer, a2a_compat::transport_from_card(&raw))?;
        return match command {
            Command::Send(cmd) => {
                client.send_message(&MessageParams::from((cmd, tenant))).await
            }
            Command::GetTask(cmd) => client.get_task(&cmd.id, cmd.history_length).await,
            Command::ListTasks(cmd) => {
                client
                    .list_tasks(cmd.context_id.as_deref(), cmd.page_size, cmd.page_token.as_deref())
                    .await
            }
            Command::CancelTask(cmd) => client.cancel_task(&cmd.id).await,
            Command::PushConfig { .. } => Err(a2a_compat::V03Error::Unsupported(
                "push-config not supported for v0.3 agents",
            )),
            Command::Stream(_) | Command::Subscribe(_) => Err(a2a_compat::V03Error::Unsupported(
                "use run_streaming() for streaming commands",
            )),
            Command::Card | Command::ExtendedCard => unreachable!("handled above"),
            Command::Agent { .. } | Command::Auth { .. } | Command::Config { .. }
            | Command::Schema { .. } | Command::GenerateSkills(_) => {
                unreachable!("management commands handled before runner")
            }
        }
        .map_err(AgcError::V03);
    }

    let card = parse_card_from_raw(&raw)?;

    match command {
        Command::Card => Ok(serde_json::to_value(&card)?),
        Command::ExtendedCard => {
            let client = build_client_from_card(&card, bearer, binding).await?;
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
            let client = build_client_from_card(&card, bearer, binding).await?;
            let result = client.send_message(&req).await;
            let resp = finish(client, result).await?;
            Ok(serde_json::to_value(&resp)?)
        }
        Command::GetTask(cmd) => {
            let client = build_client_from_card(&card, bearer, binding).await?;
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
            let client = build_client_from_card(&card, bearer, binding).await?;
            let result = client
                .list_tasks(&ListTasksRequest {
                    context_id: cmd.context_id.clone(),
                    status: cmd.status.map(TaskState::from),
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
            let client = build_client_from_card(&card, bearer, binding).await?;
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
        Command::PushConfig { command } => {
            run_push_to_value(command, &card, bearer, binding, tenant).await
        }
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

    if a2a_compat::is_v03(&raw) {
        let url = a2a_compat::rpc_url_from_card(&raw, base_url);
        let client = a2a_compat::Client::new(url, bearer, a2a_compat::transport_from_card(&raw))?;
        match command {
            Command::Stream(cmd) => {
                client.stream_message(&MessageParams::from((cmd, tenant)), on_event).await?;
            }
            Command::Subscribe(cmd) => {
                client.subscribe(&cmd.id, on_event).await?;
            }
            _ => return Err(AgcError::InvalidInput("not a streaming command".to_string())),
        }
        return Ok(());
    }

    let card = parse_card_from_raw(&raw)?;
    let mut on_event = on_event;
    match command {
        Command::Stream(cmd) => {
            let req = build_send_request(cmd, tenant);
            let client = build_client_from_card(&card, bearer, binding).await?;
            let mut stream = client
                .send_streaming_message(&req)
                .await
                .map_err(AgcError::A2A)?;
            while let Some(event) = stream.next().await {
                match event {
                    Ok(e) => on_event(serde_json::to_value(&e)?)?,
                    Err(e) => return Err(AgcError::A2A(e)),
                }
            }
            let _ = client.destroy().await;
        }
        Command::Subscribe(cmd) => {
            let client = build_client_from_card(&card, bearer, binding).await?;
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
                    Err(e) => return Err(AgcError::A2A(e)),
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

// ── Push config (v1 only) ─────────────────────────────────────────────

async fn run_push_to_value(
    command: &PushConfigCommand,
    card: &AgentCard,
    bearer: Option<&str>,
    binding: Option<Binding>,
    tenant: Option<&str>,
) -> Result<Value> {
    let client = build_client_from_card(card, bearer, binding).await?;
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

// ── Card helpers ──────────────────────────────────────────────────────

pub async fn fetch_card(base_url: &str, bearer: Option<&str>) -> Result<AgentCard> {
    let raw = fetch_card_raw(base_url, bearer).await?;
    parse_card_from_raw(&raw)
}

/// Parse a raw card JSON into an AgentCard.
/// Tries v1 deserialization first; falls back to v0.3 normalization.
fn parse_card_from_raw(raw: &Value) -> Result<AgentCard> {
    if let Ok(card) = serde_json::from_value::<AgentCard>(raw.clone()) {
        return Ok(card);
    }
    a2a_compat::normalize_card(raw).map_err(AgcError::V03)
}

/// Fetch the agent card as raw JSON — works for any protocol version.
pub async fn fetch_card_raw(base_url: &str, bearer: Option<&str>) -> Result<serde_json::Value> {
    let http = build_http_client(bearer)?;
    let url = format!("{}/.well-known/agent-card.json", base_url.trim_end_matches('/'));
    let resp = http.get(&url).send().await.map_err(AgcError::Http)?;
    if !resp.status().is_success() {
        return Err(AgcError::A2A(a2a::A2AError::internal(
            format!("agent card fetch returned HTTP {}", resp.status()),
        )));
    }
    resp.json().await.map_err(AgcError::Http)
}

pub(crate) fn build_http_client(bearer: Option<&str>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder();
    if let Some(token) = bearer {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {token}")
                .parse()
                .map_err(|e| AgcError::Auth(format!("{e}")))?,
        );
        builder = builder.default_headers(headers);
    }
    builder.build().map_err(AgcError::Http)
}

async fn build_client_from_card(
    card: &AgentCard,
    bearer: Option<&str>,
    binding: Option<Binding>,
) -> Result<A2AClient> {
    let mut builder = A2AClientFactory::builder();
    if let Some(b) = binding {
        let proto = match b {
            Binding::Jsonrpc => "JSONRPC",
            Binding::HttpJson => "HTTP+JSON",
        };
        builder = builder.preferred_bindings(vec![proto.to_string()]);
    }
    if let Some(token) = bearer {
        builder = builder.with_interceptor(Arc::new(AuthInterceptor::bearer(token)));
    }
    builder.build().create_from_card(card).await.map_err(AgcError::A2A)
}

async fn finish<T>(client: A2AClient, result: std::result::Result<T, a2a::A2AError>) -> Result<T> {
    let _ = client.destroy().await;
    result.map_err(AgcError::A2A)
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

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use a2acli::{MessageCommand, TaskIdCommand};

    // ── is_streaming ──────────────────────────────────────────────────

    #[test]
    fn is_streaming_stream_command() {
        let cmd = Command::Stream(MessageCommand {
            text: "hi".to_string(),
            context_id: None,
            task_id: None,
            history_length: None,
            accepted_output_modes: vec![],
            return_immediately: false,
        });
        assert!(is_streaming(&cmd));
    }

    #[test]
    fn is_streaming_subscribe_command() {
        assert!(is_streaming(&Command::Subscribe(TaskIdCommand {
            id: "t1".to_string()
        })));
    }

    #[test]
    fn is_streaming_send_is_false() {
        let cmd = Command::Send(MessageCommand {
            text: "hi".to_string(),
            context_id: None,
            task_id: None,
            history_length: None,
            accepted_output_modes: vec![],
            return_immediately: false,
        });
        assert!(!is_streaming(&cmd));
    }

    #[test]
    fn is_streaming_card_is_false() {
        assert!(!is_streaming(&Command::Card));
    }

    // ── build_send_request ────────────────────────────────────────────

    #[test]
    fn build_send_request_minimal() {
        let cmd = MessageCommand {
            text: "hello world".to_string(),
            context_id: None,
            task_id: None,
            history_length: None,
            accepted_output_modes: vec![],
            return_immediately: false,
        };
        let req = build_send_request(&cmd, None);
        assert_eq!(req.message.text(), Some("hello world"));
        assert!(req.configuration.is_none());
        assert!(req.tenant.is_none());
    }

    #[test]
    fn build_send_request_with_all_options() {
        let cmd = MessageCommand {
            text: "query".to_string(),
            context_id: Some("ctx-1".to_string()),
            task_id: Some("task-1".to_string()),
            history_length: Some(5),
            accepted_output_modes: vec!["text/plain".to_string()],
            return_immediately: true,
        };
        let req = build_send_request(&cmd, Some("tenant-X"));
        assert_eq!(req.message.context_id.as_deref(), Some("ctx-1"));
        assert_eq!(req.message.task_id.as_deref(), Some("task-1"));
        assert_eq!(req.tenant.as_deref(), Some("tenant-X"));
        let cfg = req.configuration.unwrap();
        assert_eq!(cfg.history_length, Some(5));
        assert_eq!(cfg.return_immediately, Some(true));
        assert_eq!(
            cfg.accepted_output_modes.as_deref(),
            Some(&["text/plain".to_string()][..])
        );
    }

    #[test]
    fn build_send_request_return_immediately_sets_configuration() {
        let cmd = MessageCommand {
            text: "ping".to_string(),
            context_id: None,
            task_id: None,
            history_length: None,
            accepted_output_modes: vec![],
            return_immediately: true,
        };
        let req = build_send_request(&cmd, None);
        let cfg = req.configuration.unwrap();
        assert_eq!(cfg.return_immediately, Some(true));
    }
}
