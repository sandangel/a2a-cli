pub mod mock_server;
pub use mock_server::{MockServer, MockVariant};

use a2acli::MessageCommand;
use agc::cli::Command;
use agc::runner::run_to_value;
use serde_json::Value;

/// Run a `send` command against `base_url` and return the raw JSON value.
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
pub async fn run_send_with_ctx(text: &str, base_url: &str, context_id: &str) -> Value {
    let cmd = Command::Send(MessageCommand {
        text: text.to_string(),
        context_id: Some(context_id.to_string()),
        task_id: None,
        history_length: None,
        accepted_output_modes: vec![],
        return_immediately: false,
    });
    run_to_value(&cmd, base_url, None, None, None)
        .await
        .expect("run_send_with_ctx failed")
}
