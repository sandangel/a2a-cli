#![warn(clippy::all)]

// ── Our own modules ───────────────────────────────────────────────────
pub mod api;
pub mod auth;
pub mod cli;
pub mod client;
pub mod commands;
pub mod config;
pub mod error;
pub mod examples; // canonical CLI examples — single source of truth for docs + tests

// ── Modules sourced from gws-cli via #[path] ─────────────────────────
// The crates.io publish script vendors these files into a temporary staging
// copy and rewrites these module declarations there. They are intentionally
// not copied into the git worktree.
#[rustfmt::skip]
#[allow(clippy::collapsible_if)]
#[path = "../../gws-cli/crates/google-workspace-cli/src/fs_util.rs"]
pub mod fs_util;

#[rustfmt::skip]
#[allow(clippy::should_implement_trait, clippy::collapsible_if)]
#[path = "../../gws-cli/crates/google-workspace-cli/src/formatter.rs"]
pub mod formatter;

pub mod printer; // print_json / print_agent_json with --fields
pub mod runner; // run_to_value / run_streaming
pub mod token_store;
pub mod validate;

pub use api::{Client, ClientBuilder, SendOptions, TaskListOptions, TaskStateArg, Transport};
