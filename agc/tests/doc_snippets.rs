//! Extract `agc` commands from markdown docs and run them against mock servers.
//!
//! Every `agc` line in a ```bash block in README.md, CONTEXT.md, and AGENTS.md
//! is categorised and tested:
//!
//!   Runnable    — concrete args, no placeholders → run against mock, assert success
//!   Skipped     — contains `<placeholder>`, mutates persistent state, or is streaming
//!
//! Agents referenced in docs (rover, team-a, team-b) are pre-registered against
//! mock servers so multi-agent commands (`--agent a --agent b`, `--all`) work too.

mod common;

use common::{MockServer, MockVariant};
use std::process::Command;
use tempfile::TempDir;

// ── Doc sources ───────────────────────────────────────────────────────

const README: &str = include_str!("../../README.md");
const CONTEXT: &str = include_str!("../../CONTEXT.md");
const AGENTS: &str = include_str!("../../AGENTS.md");

// ── Test fixture ──────────────────────────────────────────────────────

/// Fixture that starts mock servers for every agent alias used in the docs
/// and pre-registers them in an isolated config directory.
struct DocFixture {
    config_dir: TempDir,
    rover: MockServer,
    team_a: MockServer,
    team_b: MockServer,
}

impl DocFixture {
    async fn setup() -> Self {
        let config_dir = tempfile::tempdir().expect("tempdir");
        let rover = MockServer::start(MockVariant::V03Rest).await;
        let team_a = MockServer::start(MockVariant::V03Rest).await;
        let team_b = MockServer::start(MockVariant::V03Rest).await;

        let fixture = DocFixture {
            config_dir,
            rover,
            team_a,
            team_b,
        };

        // Register all three agents
        fixture.agc(&[
            "agent",
            "add",
            "rover",
            &fixture.rover.base_url,
            "--description",
            "Rover agent",
        ]);
        fixture.agc(&[
            "agent",
            "add",
            "team-a",
            &fixture.team_a.base_url,
            "--description",
            "Team A",
        ]);
        fixture.agc(&[
            "agent",
            "add",
            "team-b",
            &fixture.team_b.base_url,
            "--description",
            "Team B",
        ]);
        // Set rover as the active agent
        fixture.agc(&["agent", "use", "rover"]);

        fixture
    }

    /// Run `agc <args>` against the fixture's isolated config.
    fn agc(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_agc"))
            .args(args)
            .env("AGC_CONFIG_DIR", self.config_dir.path())
            // Clear AGC_AGENT_URL so the fixture config is used
            .env_remove("AGC_AGENT_URL")
            .output()
            .expect("run agc binary")
    }

    /// Run a raw command string (parsed from docs) against the fixture.
    fn run_cmd(&self, cmd: &str) -> std::process::Output {
        let args = shell_words(cmd);
        assert!(
            !args.is_empty() && args[0] == "agc",
            "not an agc command: {cmd}"
        );
        self.agc(&args[1..].iter().map(String::as_str).collect::<Vec<_>>())
    }
}

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
            let cmd = trimmed.split('#').next().unwrap_or("").trim();
            if cmd.starts_with("agc ") || cmd == "agc" {
                cmds.push(cmd.to_string());
            }
        }
    }
    cmds
}

fn should_skip(cmd: &str) -> Option<&'static str> {
    // Placeholders — not concrete invocations
    if cmd.contains('<') || cmd.contains('>') || cmd.contains("...") {
        return Some("placeholder");
    }
    // Pipe — shell feature, not testable as a single command
    if cmd.contains('|') {
        return Some("pipe");
    }
    // Mutates persistent state outside the fixture
    let skip_prefixes = [
        ("agc auth login", "auth (interactive)"),
        ("agc auth logout", "auth (mutates token)"),
        ("agc agent add", "agent add (mutates config)"),
        ("agc agent use", "agent use (mutates config)"),
        ("agc agent remove", "agent remove (mutates config)"),
        ("agc agent update", "agent update (mutates config)"),
        (
            "agc agent generate-skills",
            "agent generate-skills (live network)",
        ),
        ("agc push-config", "push-config (needs real task)"),
        ("agc task cancel", "task cancel (destructive)"),
        ("agc task subscribe", "task subscribe (streaming)"),
        ("agc stream", "stream (streaming)"),
        ("agc extended-card", "extended-card (needs auth)"),
    ];
    for (prefix, reason) in &skip_prefixes {
        if cmd.starts_with(prefix) {
            return Some(reason);
        }
    }
    None
}

// ── Shell word splitter ───────────────────────────────────────────────

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
    let fixture = DocFixture::setup().await;

    let all = extract_agc_commands(content);
    let mut ran = 0;
    let mut skipped = 0;
    let mut failures: Vec<String> = Vec::new();

    for cmd in &all {
        if let Some(reason) = should_skip(cmd) {
            skipped += 1;
            println!("  SKIP [{doc_name}] ({reason}): {cmd}");
            continue;
        }

        let out = fixture.run_cmd(cmd);
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            failures.push(format!(
                "  FAIL [{doc_name}]: `{cmd}`\n        stderr: {stderr}"
            ));
        }
        ran += 1;
    }

    println!(
        "[{doc_name}] {ran} ran, {skipped} skipped / {} total",
        all.len()
    );

    assert!(
        failures.is_empty(),
        "\n{} doc snippet(s) failed in {doc_name}:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
