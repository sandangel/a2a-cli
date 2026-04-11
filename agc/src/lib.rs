// ── Modules sourced from gws-cli via #[path] ─────────────────────────
pub mod auth_commands; // stub satisfying credential_store's crate::auth_commands ref

#[path = "../../gws-cli/crates/google-workspace-cli/src/fs_util.rs"]
pub mod fs_util;

#[path = "../../gws-cli/crates/google-workspace-cli/src/output.rs"]
pub mod output;

#[path = "../../gws-cli/crates/google-workspace-cli/src/credential_store.rs"]
pub mod credential_store;

// ── Our own modules ───────────────────────────────────────────────────
pub mod printer;      // print_json / print_agent_json with --fields
pub mod runner;       // run_to_value / run_streaming
pub mod cli;
pub mod commands;
pub mod config;
pub mod auth;
pub mod token_store;
pub mod client;
pub mod error;
pub mod validate;
