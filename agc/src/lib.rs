// ── Modules sourced from gws-cli via #[path] ─────────────────────────
pub mod auth_commands; // stub satisfying credential_store's crate::auth_commands ref

#[path = "../../gws-cli/crates/google-workspace-cli/src/fs_util.rs"]
pub mod fs_util;

#[allow(clippy::should_implement_trait, clippy::collapsible_if)]
#[path = "../../gws-cli/crates/google-workspace-cli/src/formatter.rs"]
pub mod formatter;

#[path = "../../gws-cli/crates/google-workspace-cli/src/output.rs"]
pub mod output;

#[allow(clippy::collapsible_if)]
#[path = "../../gws-cli/crates/google-workspace-cli/src/credential_store.rs"]
pub mod credential_store;

// ── Our own modules ───────────────────────────────────────────────────
pub mod auth;
pub mod cli;
pub mod client;
pub mod commands;
pub mod config;
pub mod error;
pub mod printer; // print_json / print_agent_json with --fields
pub mod runner; // run_to_value / run_streaming
pub mod token_store;
pub mod validate;
