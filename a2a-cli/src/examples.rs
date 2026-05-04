//! Canonical CLI example commands — single source of truth for docs and tests.
//!
//! These constants are used in two places:
//!   1. [`crate::commands::generate_skills`] — embedded verbatim into SKILL.md
//!   2. `a2a-cli/tests/integration.rs` — parsed and run against the mock server
//!
//! Changing an example here automatically updates both the generated skill
//! and the test that verifies it works.
//!
//! # Conventions
//!
//! Each example is a complete `a2a` invocation with a concrete, testable
//! message text (no `<placeholders>`). Tests replace the base URL but keep
//! everything else exactly as written here.

// ── Reading the reply ─────────────────────────────────────────────────

/// Extract only the reply artifacts — preferred for AI tools.
pub const SEND_FIELDS_ARTIFACTS: &str = r#"a2a send "Summarise this PR" --fields .artifacts"#;

/// Extract both state and reply in one call.
pub const SEND_FIELDS_STATE_AND_ARTIFACTS: &str =
    r#"a2a send "Summarise this PR" --fields "{status,artifacts}""#;

// ── Multi-turn conversation ───────────────────────────────────────────

/// Start a conversation — capture the contextId from the response.
pub const SEND_CAPTURE_CONTEXT: &str =
    r#"a2a send "My name is San." --fields "{contextId,artifacts}""#;

// ── Task management ───────────────────────────────────────────────────

/// Fetch a task by ID and extract just its state.
pub const TASK_GET_FIELDS_STATE: &str = r#"a2a task get test-task-id-42 --fields .status.state"#;

/// List tasks filtered to in-progress ones.
pub const TASK_LIST_STATUS_WORKING: &str = r#"a2a task list --status working"#;

// ── Output formatting ─────────────────────────────────────────────────

/// Compact extraction of id and status together.
pub const SEND_FIELDS_ID_STATE: &str = r#"a2a send "Hello" --fields "{id,status}""#;
