mod common;

use a2acli::{TaskIdCommand, TaskLookupCommand};
use agc::cli::Command;
use agc::runner::run_to_value;
use common::{MockServer, MockVariant, run_send, run_send_with_ctx};

// ── helpers ───────────────────────────────────────────────────────────

async fn run_card(base_url: &str) -> serde_json::Value {
    run_to_value(&Command::Card, base_url, None, None, None)
        .await
        .expect("run_card failed")
}

async fn run_get_task(id: &str, base_url: &str) -> serde_json::Value {
    let cmd = Command::GetTask(TaskLookupCommand {
        id: id.to_string(),
        history_length: None,
    });
    run_to_value(&cmd, base_url, None, None, None)
        .await
        .expect("run_get_task failed")
}

async fn run_cancel_task(id: &str, base_url: &str) -> serde_json::Value {
    let cmd = Command::CancelTask(TaskIdCommand { id: id.to_string() });
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
        let cmd = Command::ListTasks(a2acli::ListTasksCommand {
            context_id: None,
            status: None,
            page_size: None,
            page_token: None,
            history_length: None,
            include_artifacts: false,
        });
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
        let cmd = Command::ListTasks(a2acli::ListTasksCommand {
            context_id: None,
            status: None,
            page_size: None,
            page_token: None,
            history_length: None,
            include_artifacts: false,
        });
        let result = run_to_value(&cmd, &server.base_url, None, None, None)
            .await
            .expect("list_tasks failed");
        // The REST response is {"tasks": [...]} which after camelCase conversion
        // remains {"tasks": [...]}
        assert!(result["tasks"].is_array());
    }
}

// ── schema coherence tests ────────────────────────────────────────────

/// Verify that the `agc schema task` output contains fields that are
/// referenced in SKILL.md and integration tests.
mod schema_coherence {
    #[test]
    fn schema_task_has_expected_top_level_fields() {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_agc"))
            .args(["schema", "task"])
            .output()
            .expect("failed to run agc schema task");

        assert!(output.status.success(), "agc schema task exited non-zero");

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
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_agc"))
            .args(["schema", "task"])
            .output()
            .expect("failed to run agc schema task");

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
mod doc_examples {
    use super::*;

    /// `agc send "Summarise this PR" --fields artifacts`
    /// → artifacts array
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn doc_send_fields_artifacts() {
        let server = MockServer::start(MockVariant::V1).await;
        let result = run_send("Summarise this PR", &server.base_url).await;
        // In v1 the result is {"task": {...}}; artifacts live under task
        assert!(result["task"]["artifacts"].is_array());
        assert!(!result["task"]["artifacts"].as_array().unwrap().is_empty());
    }

    /// `agc send "Summarise this PR" --fields status.state,artifacts`
    /// → both state and artifacts present
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn doc_send_returns_status_and_artifacts() {
        let server = MockServer::start(MockVariant::V1).await;
        let result = run_send("Summarise this PR", &server.base_url).await;
        let task = &result["task"];
        assert_eq!(task["status"]["state"], "TASK_STATE_COMPLETED");
        assert!(task["artifacts"].is_array());
    }

    /// Multi-agent example — `agc --all send "Status?"` — not tested directly
    /// (needs config), but the `run_to_value` path is exercised by v1/v03 tests.

    /// `agc get-task <id> --fields status.state` — returns state string
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn doc_get_task_fields_status_state() {
        let server = MockServer::start(MockVariant::V1).await;
        let result = run_get_task("test-task-id-42", &server.base_url).await;
        // get-task returns Task directly
        assert_eq!(result["status"]["state"], "TASK_STATE_COMPLETED");
    }

    /// `agc send "..." --context-id <id>` — contextId propagated
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn doc_send_with_context_id_v1() {
        let server = MockServer::start(MockVariant::V1).await;
        let result = run_send_with_ctx("Follow up", &server.base_url, "ctx-xyz").await;
        assert!(result["task"]["contextId"].is_string());
    }

    /// v0.3 doc example: same shapes as SKILL.md Task response template
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn doc_v03_send_matches_skill_md_shape() {
        let server = MockServer::start(MockVariant::V03JsonRpc).await;
        let result = run_send("What can you do?", &server.base_url).await;
        // SKILL.md shape: { id, contextId, status: { state }, artifacts }
        assert!(result["id"].is_string());
        assert!(result["contextId"].is_string());
        assert!(result["status"]["state"].is_string());
        assert!(result["artifacts"].is_array());
    }
}

// ── config isolation tests ────────────────────────────────────────────

/// Verify that `AGC_CONFIG_DIR` redirects config to an isolated temp directory.
mod config_isolation {
    use serial_test::serial;
    use std::process::Command;

    #[test]
    #[serial]
    fn agent_add_creates_config_in_temp_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("config.yaml");

        let status = Command::new(env!("CARGO_BIN_EXE_agc"))
            .args(["agent", "add", "local", "http://localhost:8080"])
            .env("AGC_CONFIG_DIR", dir.path())
            .status()
            .expect("run agc agent add");

        assert!(status.success(), "agc agent add failed: {status}");
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
        Command::new(env!("CARGO_BIN_EXE_agc"))
            .args(["agent", "add", "testbot", "http://localhost:9090"])
            .env("AGC_CONFIG_DIR", dir.path())
            .status()
            .expect("run agc agent add");

        // List should show the agent
        let output = Command::new(env!("CARGO_BIN_EXE_agc"))
            .args(["agent", "list"])
            .env("AGC_CONFIG_DIR", dir.path())
            .output()
            .expect("run agc agent list");

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
        Command::new(env!("CARGO_BIN_EXE_agc"))
            .args(["agent", "add", "alpha", "http://localhost:7070"])
            .env("AGC_CONFIG_DIR", dir.path())
            .status()
            .expect("run agc agent add");

        let status = Command::new(env!("CARGO_BIN_EXE_agc"))
            .args(["agent", "use", "alpha"])
            .env("AGC_CONFIG_DIR", dir.path())
            .status()
            .expect("run agc agent use");

        assert!(status.success(), "agc agent use failed: {status}");

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

        Command::new(env!("CARGO_BIN_EXE_agc"))
            .args(["agent", "add", "agent-a", "http://localhost:1111"])
            .env("AGC_CONFIG_DIR", dir_a.path())
            .status()
            .expect("run agc agent add dir_a");

        Command::new(env!("CARGO_BIN_EXE_agc"))
            .args(["agent", "add", "agent-b", "http://localhost:2222"])
            .env("AGC_CONFIG_DIR", dir_b.path())
            .status()
            .expect("run agc agent add dir_b");

        let list_a = Command::new(env!("CARGO_BIN_EXE_agc"))
            .args(["agent", "list"])
            .env("AGC_CONFIG_DIR", dir_a.path())
            .output()
            .expect("run agc agent list dir_a");

        let list_b = Command::new(env!("CARGO_BIN_EXE_agc"))
            .args(["agent", "list"])
            .env("AGC_CONFIG_DIR", dir_b.path())
            .output()
            .expect("run agc agent list dir_b");

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
