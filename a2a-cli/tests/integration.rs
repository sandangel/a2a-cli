mod common;

use a2a_cli::cli::Command;
use a2a_cli::commands::task::TaskCommand;
use a2a_cli::runner::run_to_value;
use a2acli::{TaskIdCommand, TaskLookupCommand};
use common::{MockServer, MockVariant, run_send, run_send_with_ctx};

// ── helpers ───────────────────────────────────────────────────────────

async fn run_card(base_url: &str) -> serde_json::Value {
    run_to_value(&Command::Card, base_url, None, None, None)
        .await
        .expect("run_card failed")
}

async fn run_get_task(id: &str, base_url: &str) -> serde_json::Value {
    let cmd = Command::Task {
        command: TaskCommand::Get(TaskLookupCommand {
            id: id.to_string(),
            history_length: None,
        }),
    };
    run_to_value(&cmd, base_url, None, None, None)
        .await
        .expect("run_get_task failed")
}

async fn run_cancel_task(id: &str, base_url: &str) -> serde_json::Value {
    let cmd = Command::Task {
        command: TaskCommand::Cancel(TaskIdCommand { id: id.to_string() }),
    };
    run_to_value(&cmd, base_url, None, None, None)
        .await
        .expect("run_cancel_task failed")
}

// ── v1 tests ──────────────────────────────────────────────────────────

/// For A2A v1, `run_to_value` for Send wraps the response in
/// `{"task": {...}}` because `SendMessageResponse::Task(task)` serialises that
/// way.  State enums use ProtoJSON strings like `"TASK_STATE_COMPLETED"`.
mod v1 {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_returns_completed_task() {
        let server = MockServer::start(MockVariant::V1).await;
        let result = run_send("Hello", &server.base_url).await;
        // SendMessageResponse::Task serialises as {"task": {...}}
        assert_eq!(result["task"]["status"]["state"], "TASK_STATE_COMPLETED");
        assert_eq!(result["task"]["id"], "test-task-id-42");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_response_has_artifacts() {
        let server = MockServer::start(MockVariant::V1).await;
        let result = run_send("Hello", &server.base_url).await;
        assert!(result["task"]["artifacts"].is_array());
        assert!(!result["task"]["artifacts"].as_array().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_with_context_id() {
        let server = MockServer::start(MockVariant::V1).await;
        let result = run_send_with_ctx("Hello", &server.base_url, "my-ctx").await;
        // The mock always returns the canned task with contextId
        assert_eq!(result["task"]["contextId"], "test-ctx-id-42");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn card_returns_name() {
        let server = MockServer::start(MockVariant::V1).await;
        let result = run_card(&server.base_url).await;
        assert!(result["name"].is_string());
        assert_eq!(result["name"], "mock-rover");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_task_returns_task() {
        let server = MockServer::start(MockVariant::V1).await;
        let result = run_get_task("test-task-id-42", &server.base_url).await;
        // GetTask returns Task directly (no {"task": ...} wrapper)
        assert_eq!(result["id"], "test-task-id-42");
        assert_eq!(result["status"]["state"], "TASK_STATE_COMPLETED");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_task_returns_canceled_state() {
        let server = MockServer::start(MockVariant::V1).await;
        let result = run_cancel_task("test-task-id-42", &server.base_url).await;
        assert_eq!(result["status"]["state"], "TASK_STATE_CANCELED");
    }
}

// ── v0.3 JSON-RPC tests ───────────────────────────────────────────────

/// For v0.3, `a2a_compat::Client::call()` returns the JSON-RPC `result` field
/// directly with no additional wrapper.  State strings are lowercase (e.g.
/// `"completed"`) as defined by the mock.
mod v03_jsonrpc {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_returns_completed_task() {
        let server = MockServer::start(MockVariant::V03JsonRpc).await;
        let result = run_send("Hello", &server.base_url).await;
        assert_eq!(result["status"]["state"], "completed");
        assert_eq!(result["id"], "test-task-id-42");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_response_has_artifacts() {
        let server = MockServer::start(MockVariant::V03JsonRpc).await;
        let result = run_send("Hello", &server.base_url).await;
        assert!(result["artifacts"].is_array());
        assert!(!result["artifacts"].as_array().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_with_context_id() {
        let server = MockServer::start(MockVariant::V03JsonRpc).await;
        let result = run_send_with_ctx("Hello", &server.base_url, "my-ctx").await;
        // The mock returns the canned task — contextId is always present
        assert_eq!(result["contextId"], "test-ctx-id-42");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn card_returns_name() {
        let server = MockServer::start(MockVariant::V03JsonRpc).await;
        let result = run_card(&server.base_url).await;
        assert!(result["name"].is_string());
        assert_eq!(result["name"], "mock-eai");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_task_returns_task() {
        let server = MockServer::start(MockVariant::V03JsonRpc).await;
        let result = run_get_task("test-task-id-42", &server.base_url).await;
        assert_eq!(result["id"], "test-task-id-42");
        assert_eq!(result["status"]["state"], "completed");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_task_returns_canceled_state() {
        let server = MockServer::start(MockVariant::V03JsonRpc).await;
        let result = run_cancel_task("test-task-id-42", &server.base_url).await;
        assert_eq!(result["status"]["state"], "canceled");
    }

    /// list-tasks is not supported over v0.3 JSON-RPC.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn list_tasks_returns_unsupported_error() {
        let server = MockServer::start(MockVariant::V03JsonRpc).await;
        let cmd = Command::Task {
            command: TaskCommand::List(a2acli::ListTasksCommand {
                context_id: None,
                status: None,
                page_size: None,
                page_token: None,
                history_length: None,
                include_artifacts: false,
            }),
        };
        let result = run_to_value(&cmd, &server.base_url, None, None, None).await;
        assert!(
            result.is_err(),
            "expected Unsupported error for list-tasks over v0.3 JSON-RPC"
        );
    }
}

// ── v0.3 REST tests ───────────────────────────────────────────────────

/// For v0.3 REST, `a2a_compat::Client::rest_request()` converts
/// snake_case response keys to camelCase before returning.  So `context_id`
/// in the mock response becomes `contextId` in the returned Value.
mod v03_rest {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_returns_completed_task() {
        let server = MockServer::start(MockVariant::V03Rest).await;
        let result = run_send("Hello", &server.base_url).await;
        assert_eq!(result["status"]["state"], "completed");
        assert_eq!(result["id"], "test-task-id-42");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_response_has_artifacts() {
        let server = MockServer::start(MockVariant::V03Rest).await;
        let result = run_send("Hello", &server.base_url).await;
        assert!(result["artifacts"].is_array());
        assert!(!result["artifacts"].as_array().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_response_context_id_camel_cased() {
        let server = MockServer::start(MockVariant::V03Rest).await;
        // The mock returns snake_case `context_id`; a2a_compat converts it to
        // `contextId` before returning the Value.
        let result = run_send_with_ctx("Hello", &server.base_url, "my-ctx").await;
        assert_eq!(result["contextId"], "test-ctx-id-42");
        // The raw snake_case key must NOT be present
        assert!(result.get("context_id").is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn card_returns_name() {
        let server = MockServer::start(MockVariant::V03Rest).await;
        let result = run_card(&server.base_url).await;
        assert!(result["name"].is_string());
        assert_eq!(result["name"], "mock-eai");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_task_returns_task() {
        let server = MockServer::start(MockVariant::V03Rest).await;
        let result = run_get_task("test-task-id-42", &server.base_url).await;
        assert_eq!(result["id"], "test-task-id-42");
        assert_eq!(result["status"]["state"], "completed");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_task_returns_canceled_state() {
        let server = MockServer::start(MockVariant::V03Rest).await;
        let result = run_cancel_task("test-task-id-42", &server.base_url).await;
        assert_eq!(result["status"]["state"], "canceled");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn list_tasks_returns_tasks_array() {
        let server = MockServer::start(MockVariant::V03Rest).await;
        let cmd = Command::Task {
            command: TaskCommand::List(a2acli::ListTasksCommand {
                context_id: None,
                status: None,
                page_size: None,
                page_token: None,
                history_length: None,
                include_artifacts: false,
            }),
        };
        let result = run_to_value(&cmd, &server.base_url, None, None, None)
            .await
            .expect("list_tasks failed");
        // The REST response is {"tasks": [...]} which after camelCase conversion
        // remains {"tasks": [...]}
        assert!(result["tasks"].is_array());
    }
}

// ── schema coherence tests ────────────────────────────────────────────

/// Verify that the `a2a schema task` output contains fields that are
/// referenced in SKILL.md and integration tests.
mod schema_coherence {
    #[test]
    fn schema_task_has_expected_top_level_fields() {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_a2a"))
            .args(["schema", "task"])
            .output()
            .expect("failed to run a2a schema task");

        assert!(output.status.success(), "a2a schema task exited non-zero");

        let schema: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("schema task is not valid JSON");

        let props = &schema["properties"];
        // Top-level fields present in Task
        assert!(props["id"].is_object(), "schema missing 'id'");
        assert!(props["contextId"].is_object(), "schema missing 'contextId'");
        assert!(props["status"].is_object(), "schema missing 'status'");
        assert!(props["artifacts"].is_object(), "schema missing 'artifacts'");
    }

    #[test]
    fn schema_task_status_refs_task_status_def() {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_a2a"))
            .args(["schema", "task"])
            .output()
            .expect("failed to run a2a schema task");

        let schema: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("schema task is not valid JSON");

        // `status` uses a $ref to the TaskStatus definition
        assert!(
            schema["properties"]["status"]["$ref"].is_string(),
            "status should have a $ref"
        );

        // TaskStatus definition must have a `state` property with enum values
        let task_status_def = &schema["$defs"]["TaskStatus"];
        assert!(
            task_status_def["properties"]["state"]["enum"].is_array(),
            "TaskStatus.state must be an enum: {task_status_def}"
        );
        let enum_vals = task_status_def["properties"]["state"]["enum"]
            .as_array()
            .unwrap();
        let has_completed = enum_vals
            .iter()
            .any(|v| v.as_str() == Some("TASK_STATE_COMPLETED"));
        assert!(
            has_completed,
            "TaskStatus.state enum missing TASK_STATE_COMPLETED"
        );
    }
}

// ── doc-example tests ─────────────────────────────────────────────────

/// Mirror the key code examples from SKILL.md.
/// Tests derived from `a2a_cli::examples` constants — the same strings embedded in
/// SKILL.md are parsed here and run against the mock server.  If an example in
/// `src/examples.rs` is changed the test will catch any breakage automatically.
mod doc_examples {
    use super::*;
    use a2a_cli::examples;

    /// Parse `text` and `--fields <f>` out of a canonical example string like
    /// `a2a send "some text" --fields foo,bar`.
    fn parse_send_example(example: &str) -> (&str, Option<&str>) {
        // extract quoted text
        let text_start = example.find('"').unwrap() + 1;
        let text_end = example[text_start..].find('"').unwrap() + text_start;
        let text = &example[text_start..text_end];
        // extract --fields value if present
        let fields = example
            .find("--fields ")
            .map(|i| example[i + "--fields ".len()..].trim());
        (text, fields)
    }

    // ── examples::SEND_FIELDS_ARTIFACTS ──────────────────────────────────
    // Source: a2a send "Summarise this PR" --fields artifacts
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_fields_artifacts_v03() {
        let (text, _) = parse_send_example(examples::SEND_FIELDS_ARTIFACTS);
        let server = MockServer::start(MockVariant::V03JsonRpc).await;
        let result = run_send(text, &server.base_url).await;
        // v0.3: artifacts at top level
        assert!(
            result["artifacts"].is_array(),
            "expected artifacts array, got: {result}"
        );
        assert!(!result["artifacts"].as_array().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_fields_artifacts_v1() {
        let (text, _) = parse_send_example(examples::SEND_FIELDS_ARTIFACTS);
        let server = MockServer::start(MockVariant::V1).await;
        let result = run_send(text, &server.base_url).await;
        // v1: wrapped in {"task": {...}}
        assert!(result["task"]["artifacts"].is_array());
        assert!(!result["task"]["artifacts"].as_array().unwrap().is_empty());
    }

    // ── examples::SEND_FIELDS_STATE_AND_ARTIFACTS ────────────────────────
    // Source: a2a send "Summarise this PR" --fields status.state,artifacts
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_fields_state_and_artifacts_v03() {
        let (text, _) = parse_send_example(examples::SEND_FIELDS_STATE_AND_ARTIFACTS);
        let server = MockServer::start(MockVariant::V03JsonRpc).await;
        let result = run_send(text, &server.base_url).await;
        assert!(result["status"]["state"].is_string());
        assert!(result["artifacts"].is_array());
    }

    // ── examples::TASK_GET_FIELDS_STATE ──────────────────────────────────
    // Source: a2a task get test-task-id-42 --fields status.state
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn task_get_fields_state_v03() {
        let server = MockServer::start(MockVariant::V03JsonRpc).await;
        let result = run_get_task("test-task-id-42", &server.base_url).await;
        // state is a string
        assert!(result["status"]["state"].is_string());
    }

    // ── SKILL.md freshness check ──────────────────────────────────────────
    // Fails if committed skills/a2a/SKILL.md differs from `a2a generate-skills`
    // output, catching any time generate_skills.rs is edited without regenerating.
    #[test]
    fn skill_md_is_up_to_date() {
        use std::process::Command;

        let dir = tempfile::tempdir().expect("tempdir");
        let status = Command::new(env!("CARGO_BIN_EXE_a2a"))
            .args(["generate-skills"])
            .current_dir(dir.path())
            .status()
            .expect("run a2a generate-skills");
        assert!(status.success(), "a2a generate-skills failed");

        let generated = std::fs::read_to_string(dir.path().join("skills/a2a/SKILL.md"))
            .expect("read generated SKILL.md");

        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let committed = std::fs::read_to_string(workspace.join("skills/a2a/SKILL.md"))
            .expect("read committed skills/a2a/SKILL.md");

        assert_eq!(
            committed, generated,
            "\nskills/a2a/SKILL.md is stale — run `a2a generate-skills` to update it"
        );
    }
}

// ── config isolation tests ────────────────────────────────────────────

/// Verify that `A2A_CONFIG_DIR` redirects config to an isolated temp directory.
mod config_isolation {
    use serial_test::serial;
    use std::process::Command;

    #[test]
    #[serial]
    fn agent_add_creates_config_in_temp_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("config.yaml");

        let status = Command::new(env!("CARGO_BIN_EXE_a2a"))
            .args(["agent", "add", "local", "http://localhost:8080"])
            .env("A2A_CONFIG_DIR", dir.path())
            .status()
            .expect("run a2a agent add");

        assert!(status.success(), "a2a agent add failed: {status}");
        assert!(
            config_path.exists(),
            "config.yaml not created at {config_path:?}"
        );
    }

    #[test]
    #[serial]
    fn agent_list_reads_from_temp_dir() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Add an agent first
        Command::new(env!("CARGO_BIN_EXE_a2a"))
            .args(["agent", "add", "testbot", "http://localhost:9090"])
            .env("A2A_CONFIG_DIR", dir.path())
            .status()
            .expect("run a2a agent add");

        // List should show the agent
        let output = Command::new(env!("CARGO_BIN_EXE_a2a"))
            .args(["agent", "list"])
            .env("A2A_CONFIG_DIR", dir.path())
            .output()
            .expect("run a2a agent list");

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("testbot"),
            "expected 'testbot' in agent list, got: {stdout}"
        );
    }

    #[test]
    #[serial]
    fn agent_use_updates_current_agent() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Add and then switch to the agent
        Command::new(env!("CARGO_BIN_EXE_a2a"))
            .args(["agent", "add", "alpha", "http://localhost:7070"])
            .env("A2A_CONFIG_DIR", dir.path())
            .status()
            .expect("run a2a agent add");

        let status = Command::new(env!("CARGO_BIN_EXE_a2a"))
            .args(["agent", "use", "alpha"])
            .env("A2A_CONFIG_DIR", dir.path())
            .status()
            .expect("run a2a agent use");

        assert!(status.success(), "a2a agent use failed: {status}");

        // Verify config persists the current_agent
        let config_text =
            std::fs::read_to_string(dir.path().join("config.yaml")).expect("read config");
        assert!(
            config_text.contains("current_agent") || config_text.contains("alpha"),
            "config does not contain active agent: {config_text}"
        );
    }

    #[test]
    #[serial]
    fn separate_temp_dirs_are_isolated() {
        let dir_a = tempfile::tempdir().expect("tempdir a");
        let dir_b = tempfile::tempdir().expect("tempdir b");

        Command::new(env!("CARGO_BIN_EXE_a2a"))
            .args(["agent", "add", "agent-a", "http://localhost:1111"])
            .env("A2A_CONFIG_DIR", dir_a.path())
            .status()
            .expect("run a2a agent add dir_a");

        Command::new(env!("CARGO_BIN_EXE_a2a"))
            .args(["agent", "add", "agent-b", "http://localhost:2222"])
            .env("A2A_CONFIG_DIR", dir_b.path())
            .status()
            .expect("run a2a agent add dir_b");

        let list_a = Command::new(env!("CARGO_BIN_EXE_a2a"))
            .args(["agent", "list"])
            .env("A2A_CONFIG_DIR", dir_a.path())
            .output()
            .expect("run a2a agent list dir_a");

        let list_b = Command::new(env!("CARGO_BIN_EXE_a2a"))
            .args(["agent", "list"])
            .env("A2A_CONFIG_DIR", dir_b.path())
            .output()
            .expect("run a2a agent list dir_b");

        let stdout_a = String::from_utf8_lossy(&list_a.stdout);
        let stdout_b = String::from_utf8_lossy(&list_b.stdout);

        assert!(
            stdout_a.contains("agent-a"),
            "dir_a list missing agent-a: {stdout_a}"
        );
        assert!(
            !stdout_a.contains("agent-b"),
            "dir_a list should not contain agent-b"
        );
        assert!(
            stdout_b.contains("agent-b"),
            "dir_b list missing agent-b: {stdout_b}"
        );
        assert!(
            !stdout_b.contains("agent-a"),
            "dir_b list should not contain agent-a"
        );
    }
}

// ── rename regression tests ──────────────────────────────────────────

mod rename_regression {
    const README: &str = include_str!("../../README.md");
    const AGENTS: &str = include_str!("../../AGENTS.md");
    const CONTRIBUTING: &str = include_str!("../../CONTRIBUTING.md");
    const CONFIG_EXAMPLE: &str = include_str!("../../config.example.yaml");
    const DOC_SNIPPETS_TEST: &str = include_str!("doc_snippets.rs");
    const COMMON_TEST: &str = include_str!("common/mod.rs");

    #[test]
    fn docs_do_not_use_old_command_in_bash_snippets() {
        for (name, content) in [
            ("README.md", README),
            ("AGENTS.md", AGENTS),
            ("CONTRIBUTING.md", CONTRIBUTING),
        ] {
            for (line_no, line) in bash_lines(content) {
                let trimmed = line.trim();
                assert!(
                    trimmed != "a2a-cli" && !trimmed.starts_with("a2a-cli "),
                    "{name}:{line_no} uses package name as a command: {trimmed}"
                );
                assert!(
                    !trimmed.contains("-p a2a "),
                    "{name}:{line_no} uses binary name as Rust package: {trimmed}"
                );
                assert!(
                    !trimmed.contains("_a2a-cli") && !trimmed.contains("a2a-cli.fish"),
                    "{name}:{line_no} uses package name as completion artifact: {trimmed}"
                );
            }
        }
    }

    #[test]
    fn owned_docs_do_not_use_old_package_or_skill_names() {
        for (name, content) in [
            ("README.md", README),
            ("AGENTS.md", AGENTS),
            ("CONTRIBUTING.md", CONTRIBUTING),
            ("config.example.yaml", CONFIG_EXAMPLE),
        ] {
            assert!(
                content.contains("a2a"),
                "{name} is missing current command/package text"
            );
        }

        assert!(README.contains("@rover/a2a-cli"));
        assert!(AGENTS.contains("skills/a2a/SKILL.md"));
        assert!(CONTRIBUTING.contains("a2a-cli/"));
    }

    #[test]
    fn rust_doc_snippet_tests_use_renamed_binary_crate_and_skill_path() {
        assert!(DOC_SNIPPETS_TEST.contains("CARGO_BIN_EXE_a2a"));
        assert!(DOC_SNIPPETS_TEST.contains("skills/a2a/SKILL.md"));
        assert!(COMMON_TEST.contains("a2a_cli::"));
    }

    fn bash_lines(markdown: &str) -> impl Iterator<Item = (usize, &str)> + '_ {
        let mut in_bash = false;
        markdown.lines().enumerate().filter_map(move |(idx, line)| {
            let trimmed = line.trim();
            if trimmed.starts_with("```bash") || trimmed.starts_with("``` bash") {
                in_bash = true;
                return None;
            }
            if trimmed == "```" && in_bash {
                in_bash = false;
                return None;
            }
            in_bash.then_some((idx + 1, line))
        })
    }
}
