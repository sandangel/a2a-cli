//! Extract `agc` commands from markdown docs and run them against the mock server.
//!
//! Every `agc` line in a ```bash block in README.md, CONTEXT.md, and AGENTS.md
//! is categorised and tested:
//!
//!   Runnable    — concrete args, no placeholders → run against mock, assert success
//!   Skipped     — contains `<placeholder>`, requires live auth, or modifies config
//!
//! This ensures docs stay coherent with the binary as the CLI evolves.
//! To add a new doc example: write it in the markdown, the test picks it up.

mod common;

use common::{MockServer, MockVariant};
use std::process::Command;

// ── Doc sources ───────────────────────────────────────────────────────

const README: &str = include_str!("../../README.md");
const CONTEXT: &str = include_str!("../../CONTEXT.md");
const AGENTS: &str = include_str!("../../AGENTS.md");

// ── Snippet extraction ────────────────────────────────────────────────

fn extract_agc_commands(markdown: &str) -> Vec<String> {
    let mut cmds = Vec::new();
    let mut in_bash = false;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```bash") || trimmed.starts_with("``` bash") {
            in_bash = true;
            continue;
        }
        if trimmed == "```" && in_bash {
            in_bash = false;
            continue;
        }
        if in_bash {
            // strip inline comment, trim
            let cmd = trimmed.split('#').next().unwrap_or("").trim();
            if cmd.starts_with("agc ") || cmd == "agc" {
                cmds.push(cmd.to_string());
            }
        }
    }
    cmds
}

/// Commands that can be run against the mock server.
/// Returns `None` if the command should be skipped.
fn classify(cmd: &str) -> Option<MockVariant> {
    // Skip placeholders
    if cmd.contains('<') || cmd.contains('>') || cmd.contains("...") {
        return None;
    }
    // Skip pipe expressions
    if cmd.contains('|') {
        return None;
    }
    // Skip multi-agent (needs pre-registered aliases)
    if cmd.matches("--agent").count() > 1 || cmd.contains("--all") {
        return None;
    }
    // Skip commands that touch real config, auth, or require live network
    let skip_prefixes = [
        "agc auth login",
        "agc auth logout",
        "agc agent add",
        "agc agent use",
        "agc agent remove",
        "agc agent update",
        "agc agent generate-skills",
        "agc push-config",
        "agc task cancel",    // destructive
        "agc task subscribe", // streaming, needs real agent
        "agc task list",      // unsupported over v0.3 JSON-RPC (tested separately)
        "agc stream",         // streaming
        "agc extended-card",  // needs auth
    ];
    for prefix in &skip_prefixes {
        if cmd.starts_with(prefix) {
            return None;
        }
    }
    Some(MockVariant::V03JsonRpc)
}

// ── Runner ────────────────────────────────────────────────────────────

fn run_cmd(cmd: &str, agent_url: &str) -> std::process::Output {
    // Split the command string into args (handles quoted strings naively)
    let args = shell_words(cmd);
    assert!(!args.is_empty() && args[0] == "agc");

    Command::new(env!("CARGO_BIN_EXE_agc"))
        .args(&args[1..])
        .env("AGC_AGENT_URL", agent_url)
        // Isolate config so tests never touch the developer's real config
        .env("AGC_CONFIG_DIR", std::env::temp_dir().join("agc-doc-test"))
        .output()
        .expect("failed to run agc binary")
}

/// Minimal shell word splitter: handles `"quoted strings"` and bare words.
fn shell_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in s.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ' ' | '\t' if !in_quotes => {
                if !current.is_empty() {
                    words.push(current.clone());
                    current.clear();
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

// ── Tests ─────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn readme_snippets_run_against_mock() {
    run_doc_snippets("README.md", README).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn context_snippets_run_against_mock() {
    run_doc_snippets("CONTEXT.md", CONTEXT).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agents_snippets_run_against_mock() {
    run_doc_snippets("AGENTS.md", AGENTS).await;
}

async fn run_doc_snippets(doc_name: &str, content: &str) {
    let server = MockServer::start(MockVariant::V03JsonRpc).await;

    let all = extract_agc_commands(content);
    let mut ran = 0;
    let mut skipped = 0;
    let mut failures: Vec<String> = Vec::new();

    for cmd in &all {
        match classify(cmd) {
            None => {
                skipped += 1;
            }
            Some(_) => {
                let out = run_cmd(cmd, &server.base_url);
                if !out.status.success() {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    failures.push(format!(
                        "  FAIL [{doc_name}]: `{cmd}`\n        stderr: {stderr}"
                    ));
                }
                ran += 1;
            }
        }
    }

    println!(
        "[{doc_name}] {ran} commands run, {skipped} skipped out of {} total",
        all.len()
    );

    assert!(
        failures.is_empty(),
        "\n{} doc snippet(s) failed in {doc_name}:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
