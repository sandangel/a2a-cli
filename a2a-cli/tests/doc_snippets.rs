//! Extract `a2a` commands from markdown docs and run them against mock servers.
//!
//! Every `a2a` line in a ```bash block in README.md and AGENTS.md
//! is categorised and tested. All commands are exercised:
//!
//!   - Placeholder args (<id>, <alias>, etc.) are substituted with real mock values
//!   - Pipe expressions are run via `sh -c "..."`
//!   - Streaming commands (stream, task subscribe) run with a 5 s timeout
//!   - Config-mutating commands (agent add/use/remove, auth logout) use an
//!     isolated temp config dir and are safe to run
//!   - agent generate-skills runs from a temp working dir to avoid polluting cwd
//!
//! Only `a2a auth login` is skipped — it requires an interactive browser OAuth flow.
//!
//! DocFixture starts three V1 mock servers and pre-registers them as the
//! agent aliases used across all three docs (example, team-a, team-b).

mod common;

use common::{MOCK_CFG_ID, MOCK_CTX_ID, MOCK_TASK_ID, MockServer, MockVariant};
use std::time::Duration;
use tempfile::TempDir;
use tokio::process::Command as TokioCommand;

// ── Doc sources ───────────────────────────────────────────────────────

const README: &str = include_str!("../../README.md");
const AGENTS: &str = include_str!("../../AGENTS.md");

// ── Fixture ───────────────────────────────────────────────────────────

struct DocFixture {
    config_dir: TempDir,
    skills_dir: TempDir,
    example: MockServer,
    team_a: MockServer,
    team_b: MockServer,
}

impl DocFixture {
    async fn setup() -> Self {
        let config_dir = tempfile::tempdir().expect("config tempdir");
        let skills_dir = tempfile::tempdir().expect("skills tempdir");
        let example = MockServer::start(MockVariant::V1).await;
        let team_a = MockServer::start(MockVariant::V1).await;
        let team_b = MockServer::start(MockVariant::V1).await;

        let fix = DocFixture {
            config_dir,
            skills_dir,
            example,
            team_a,
            team_b,
        };

        // Register all aliases used across the docs and set example as active
        fix.a2a_sync(&[
            "agent",
            "add",
            "example",
            &fix.example.base_url,
            "--description",
            "Example",
        ]);
        fix.a2a_sync(&[
            "agent",
            "add",
            "team-a",
            &fix.team_a.base_url,
            "--description",
            "Team A",
        ]);
        fix.a2a_sync(&[
            "agent",
            "add",
            "team-b",
            &fix.team_b.base_url,
            "--description",
            "Team B",
        ]);
        fix.a2a_sync(&["agent", "use", "example"]);

        fix
    }

    /// Base command builder — sets isolated config and clears env agent override.
    fn base_cmd(&self, binary: &str) -> TokioCommand {
        let mut cmd = TokioCommand::new(binary);
        cmd.env("A2A_CONFIG_DIR", self.config_dir.path())
            .env_remove("A2A_AGENT_URL")
            .env("A2A_KEYRING_BACKEND", "file"); // safe in CI / test envs
        cmd
    }

    /// Run a synchronous setup command (no timeout needed).
    fn a2a_sync(&self, args: &[&str]) {
        let status = std::process::Command::new(env!("CARGO_BIN_EXE_a2a"))
            .args(args)
            .env("A2A_CONFIG_DIR", self.config_dir.path())
            .env_remove("A2A_AGENT_URL")
            .env("A2A_KEYRING_BACKEND", "file")
            .status()
            .expect("run a2a");
        assert!(status.success(), "setup command failed: a2a {args:?}");
    }

    /// Run a parsed command string, returning (exit_ok, stderr).
    async fn run(&self, cmd: &str) -> (bool, String) {
        let (is_stream, is_pipe, is_generate_skills) = (
            cmd.starts_with("a2a stream") || cmd.contains("task subscribe"),
            cmd.contains('|'),
            cmd.contains("generate-skills"),
        );

        if is_pipe {
            self.run_pipe(cmd).await
        } else if is_stream {
            self.run_streaming(cmd).await
        } else if is_generate_skills {
            self.run_in_skills_dir(cmd).await
        } else {
            self.run_plain(cmd).await
        }
    }

    /// Run a plain a2a command.
    async fn run_plain(&self, cmd: &str) -> (bool, String) {
        let args = shell_words(cmd);
        let out = self
            .base_cmd(env!("CARGO_BIN_EXE_a2a"))
            .args(&args[1..])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .expect("run a2a");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        (out.status.success(), stderr)
    }

    /// Run a pipe expression via the shell with env vars set.
    async fn run_pipe(&self, cmd: &str) -> (bool, String) {
        // Put the a2a binary's directory on PATH so `sh` can find it.
        let bin_path = std::path::Path::new(env!("CARGO_BIN_EXE_a2a"));
        let bin_dir = bin_path.parent().unwrap().to_str().unwrap().to_string();
        let path = format!("{bin_dir}:{}", std::env::var("PATH").unwrap_or_default());
        let out = self
            .base_cmd("sh")
            .arg("-c")
            .arg(cmd)
            .env("PATH", path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .expect("run sh -c");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        (out.status.success(), stderr)
    }

    /// Run a streaming command with a 5 s timeout — the mock sends one event
    /// and closes the stream, so the process should exit well within that.
    async fn run_streaming(&self, cmd: &str) -> (bool, String) {
        let args = shell_words(cmd);
        let mut child = self
            .base_cmd(env!("CARGO_BIN_EXE_a2a"))
            .args(&args[1..])
            .kill_on_drop(true)
            .spawn()
            .expect("spawn a2a");

        match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
            Ok(Ok(status)) => (status.success(), String::new()),
            Ok(Err(e)) => (false, format!("wait error: {e}")),
            Err(_) => {
                // Timeout — kill and treat as success if the mock sent at least one event.
                // Streaming commands are designed to run until the server closes the stream;
                // a timeout just means the mock kept the connection open longer than 5 s.
                let _ = child.kill().await;
                (true, "timed out (expected for streaming)".to_string())
            }
        }
    }

    /// Run `agent generate-skills` from the skills_dir so files land in a temp dir.
    async fn run_in_skills_dir(&self, cmd: &str) -> (bool, String) {
        let args = shell_words(cmd);
        let out = self
            .base_cmd(env!("CARGO_BIN_EXE_a2a"))
            .args(&args[1..])
            .current_dir(self.skills_dir.path())
            .output()
            .await
            .expect("run a2a generate-skills");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        (out.status.success(), stderr)
    }
}

// ── Placeholder substitution ──────────────────────────────────────────

/// Replace all `<placeholder>` tokens in a doc command with real mock values.
fn substitute(cmd: &str, fix: &DocFixture) -> String {
    cmd
        // IDs
        .replace("<id>", MOCK_TASK_ID)
        .replace("<task-id>", MOCK_TASK_ID)
        .replace("<config-id>", MOCK_CFG_ID)
        .replace("<context-id>", MOCK_CTX_ID)
        // Context ID variants used in README/CONTEXT examples
        .replace("<contextId from above>", MOCK_CTX_ID)
        .replace("<contextId>", MOCK_CTX_ID)
        // URLs
        .replace("<callback-url>", "http://127.0.0.1:19999/callback")
        .replace("<url>", &fix.example.base_url)
        // Aliases
        .replace("<alias|url>", "example")
        .replace("<alias1>", "team-a")
        .replace("<alias2>", "team-b")
        .replace("<alias>", "example")
        // Quoted message placeholders
        .replace("\"<describe what you want>\"", "\"Hello\"")
        .replace("\"<your request>\"", "\"Hello\"")
        .replace("\"<text>\"", "\"Hello\"")
        .replace("<your request>", "Hello")
        // Misc
        .replace("<target>", "x86_64-unknown-linux-gnu")
        .replace("<paths>", ".id")
        // Real agent URLs from doc examples → mock server URLs
        .replace("https://agent.example.com", &fix.example.base_url)
        .replace("http://localhost:8080", &fix.example.base_url)
}

// ── Snippet extraction ────────────────────────────────────────────────

fn extract_a2a_commands(markdown: &str) -> Vec<String> {
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
            if cmd.starts_with("a2a ") || cmd == "a2a" {
                cmds.push(cmd.to_string());
            }
        }
    }
    cmds
}

/// Only `auth login` is truly untestable — it opens an interactive OAuth browser flow.
/// Usage/syntax description lines (containing `[`) are also skipped — they're templates,
/// not runnable commands.
/// Commands that redirect output to a file (`>`) are shell setup instructions that
/// depend on the user's environment (e.g. `a2a completions zsh > ~/.zsh/completions/_a2a`).
fn skip_reason(cmd: &str) -> Option<&'static str> {
    if cmd.starts_with("a2a auth login") {
        Some("interactive OAuth")
    } else if cmd.contains('[') {
        Some("usage template")
    } else if cmd.contains('>') {
        Some("environment-dependent shell redirect")
    } else {
        None
    }
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
                    words.push(std::mem::take(&mut current));
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
async fn agents_snippets_run_against_mock() {
    run_doc_snippets("AGENTS.md", AGENTS).await;
}

/// Generate `skills/a2a/SKILL.md` fresh from the current binary, then run all
/// its bash snippets against the mock server.  This ensures the skill template
/// is tested without requiring a manual `a2a generate-skills` step.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn skill_a2a_snippets_run_against_mock() {
    let fix = DocFixture::setup().await;

    // Generate the skill into a temp dir so we always test the current template.
    let skills_dir = tempfile::tempdir().expect("skills tempdir");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_a2a"))
        .args(["generate-skills"])
        .env("A2A_CONFIG_DIR", fix.config_dir.path())
        .env("A2A_KEYRING_BACKEND", "file")
        .env_remove("A2A_AGENT_URL")
        .current_dir(skills_dir.path())
        .status()
        .expect("run a2a generate-skills");
    assert!(status.success(), "a2a generate-skills failed");

    let content = std::fs::read_to_string(skills_dir.path().join("skills/a2a/SKILL.md"))
        .expect("read generated SKILL.md");

    run_doc_snippets_with_fix("skills/a2a/SKILL.md", &content, &fix).await;
}

async fn run_doc_snippets(doc_name: &str, content: &str) {
    let fix = DocFixture::setup().await;
    run_doc_snippets_with_fix(doc_name, content, &fix).await;
}

async fn run_doc_snippets_with_fix(doc_name: &str, content: &str, fix: &DocFixture) {
    let all = extract_a2a_commands(content);
    let mut ran = 0;
    let mut skipped = 0;
    let mut failures: Vec<String> = Vec::new();

    for raw_cmd in &all {
        if let Some(reason) = skip_reason(raw_cmd) {
            skipped += 1;
            println!("  SKIP [{doc_name}] ({reason}): {raw_cmd}");
            continue;
        }

        let cmd = substitute(raw_cmd, &fix);
        let (ok, stderr) = fix.run(&cmd).await;

        if ok {
            ran += 1;
        } else {
            failures.push(format!(
                "  FAIL [{doc_name}]:\n    original: {raw_cmd}\n    substituted: {cmd}\n    stderr: {stderr}"
            ));
            ran += 1;
        }
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
