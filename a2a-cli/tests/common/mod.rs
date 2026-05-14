pub mod mock_server;
#[allow(unused_imports)]
pub use mock_server::{
    MOCK_CFG_ID, MOCK_CTX_ID, MOCK_FOLLOW_UP_TEXT, MOCK_TASK_ID, MockServer, MockVariant,
};

use a2a_cli::cli::Command;
use a2a_cli::runner::run_to_value;
use a2acli::MessageCommand;
use serde_json::Value;

/// Run a `send` command against `base_url` and return the raw JSON value.
#[allow(dead_code)]
pub async fn run_send(text: &str, base_url: &str) -> Value {
    let cmd = Command::Send(MessageCommand {
        text: text.to_string(),
        context_id: None,
        task_id: None,
        history_length: None,
        accepted_output_modes: vec![],
        return_immediately: false,
    });
    run_to_value(&cmd, base_url, None, None, None)
        .await
        .expect("run_send failed")
}

/// Run a `send` command with an explicit context_id and return the raw JSON value.
#[allow(dead_code)]
pub async fn run_send_with_ctx(text: &str, base_url: &str, context_id: &str) -> Value {
    try_send_with_ctx(text, base_url, context_id)
        .await
        .expect("run_send_with_ctx failed")
}

/// Run a `send` command with an explicit context_id and return the result.
#[allow(dead_code)]
pub async fn try_send_with_ctx(
    text: &str,
    base_url: &str,
    context_id: &str,
) -> a2a_cli::error::Result<Value> {
    let cmd = Command::Send(MessageCommand {
        text: text.to_string(),
        context_id: Some(context_id.to_string()),
        task_id: None,
        history_length: None,
        accepted_output_modes: vec![],
        return_immediately: false,
    });
    run_to_value(&cmd, base_url, None, None, None).await
}
