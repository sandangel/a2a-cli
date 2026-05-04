# AGENTS.md

## Project Overview

`a2a-cli` is a **Rust** CLI for interacting with agents that implement the [A2A protocol](https://a2aproject.github.io/A2A/). It is published to GitHub Packages as `@sandangel/a2a-protocol-cli`, with Rust `.crate` archives attached to GitHub Releases. The Rust package is `a2a-protocol-cli`, and the library crate is `a2a_cli`.

> [!IMPORTANT]
> This CLI is designed to be invoked by AI coding tools (Claude Code, Copilot, Cursor, etc.) as well as humans. Always assume CLI argument inputs can be adversarial — validate paths, reject control characters and dangerous Unicode, and encode user values before embedding in URLs or filenames. See `a2a-cli/src/validate.rs`.

> [!NOTE]
> **Reference repos** (`A2A/`, `a2a-go/`, `gws-cli/`) are read-only references. Do not modify them.
> `a2a-rs/` is also read-only — it is a git submodule containing the published A2A Rust SDK.

## Rules of Engagement for AI Agents

- **Read the answer from `artifacts`** — per the A2A spec, task outputs MUST be returned in `artifacts`. `status.message` is for in-progress communication only (e.g. `input-required` prompts), not final results.
- **Use `--fields .artifacts`** for concise extraction of the reply; use `--format table` for human-readable output.
- **Check `status.state`** to understand task state: `completed`, `input-required`, `failed`, etc.
- **Never expose tokens** — bearer tokens and client secrets are sensitive; use keychain storage.
- **Confirm before canceling tasks** — `a2a task cancel` is destructive.
- **Use `a2a schema`** to inspect data structures before crafting messages.

## Build & Test

```bash
uv run inv build          # dev build
uv run inv release        # release build
uv run inv test           # run all tests
uv run inv lint           # fmt check + clippy
uv run inv fix            # auto-fix fmt + clippy

# Output binary: target/debug/a2a (dev) or target/release/a2a (release)
```

Rust edition: 2024. Minimum Rust version: 1.85. Use `uv run inv` for all local tasks — see `tasks.py`.

## Quick Start

```bash
# Register an agent
a2a agent add example https://agent.example.com
a2a agent use example

# Authenticate (auto-detects OAuth flow from agent card)
a2a auth login

# Send a message
a2a send "Hello, agent!"

# Get just the reply artifacts
a2a send "What is the status?" --fields .artifacts

# Multi-turn conversation
a2a send "My name is San." --fields "{contextId,artifacts}"
a2a send "What is my name?" --context-id <contextId from above>
```

## Architecture

### Commands

```bash
a2a [--agent <alias|url>] [--format json|table|yaml|csv] [--fields <jq>] [--compact] <command> [args]
```

| Command | Purpose |
|---------|---------|
| `a2a send` | Send a message to one or more agents and wait for response |
| `a2a stream` | Send a streaming message — prints events as they arrive |
| `a2a card` | Fetch and display the public agent card |
| `a2a extended-card` | Fetch the authenticated extended agent card |
| `a2a task get <id>` | Fetch a task by ID |
| `a2a task list` | List tasks with optional filters |
| `a2a task cancel <id>` | Cancel a running task |
| `a2a task subscribe <id>` | Subscribe to live task updates (streaming) |
| `a2a push-config create/get/list/delete` | Manage push notification configs |
| `a2a agent add/use/list/remove/show/update` | Named agent alias registry |
| `a2a agent generate-skills [alias...]` | Generate `skills/<alias>/SKILL.md` from live agent card |
| `a2a auth login/logout/status` | Per-agent OAuth flows |
| `a2a schema send/task/card` | Inspect A2A protocol types (JSON Schema generated from proto) |
| `a2a config show` | Show CLI configuration |
| `a2a generate-skills` | Regenerate `skills/a2a/SKILL.md` — a2a CLI reference for LLMs |
| `a2a completions <shell>` | Print shell completion script (bash, zsh, fish, elvish, powershell) |

### Global Flags

| Flag | Description |
|------|-------------|
| `--agent <alias\|url>` | Target agent (repeatable for parallel multi-agent) |
| `--all` | All registered agents in parallel |
| `--format json\|table\|yaml\|csv` | Output format (default: `json`; use `table` for human-readable) |
| `--compact` | Single-line JSON (only with `--format json`) |
| `--fields <jq>` | jq filter applied to output (e.g. `.artifacts[0]`); AI tools |
| `--transport jsonrpc\|http-json` | Override transport (default: auto from agent card) |
| `--tenant <id>` | Optional tenant ID forwarded to A2A requests |
| `--bearer-token <token>` | Static bearer token, bypasses OAuth |

### Output

```bash
# Human-readable
a2a --format table agent list
a2a --format table auth status

# AI tools — extract just what you need
a2a send "Hello" --fields .artifacts        # task output
a2a send "Hello" --fields .status.state     # just the state
a2a send "Hello" --compact                  # single-line JSON
```

Multi-agent output is always NDJSON — one compact JSON line per agent, each tagged with `agent` and `agent_url`:

```bash
a2a --all send "Status?" | jq -r '"[\(.agent)] \(.status.state)"'
```

### Source Layout

All implementation is currently under `a2a-cli/src/`. Key modules:

| Module | Purpose |
|--------|---------|
| `a2a-cli/src/main.rs` | Entry point — CLI parse, dispatch, multi-agent orchestration |
| `a2a-cli/src/cli.rs` | `Cli`, `GlobalArgs`, `Command` enum (clap derive) |
| `a2a-cli/src/runner.rs` | `run_to_value` / `run_streaming` — A2A protocol dispatch |
| `a2a-cli/src/client.rs` | Agent resolution, `build_http_client`, `resolve_target` |
| `a2a-cli/src/config.rs` | Config file model (`~/.config/a2a-cli/config.yaml`) |
| `a2a-cli/src/auth.rs` | OAuth flow selection, PKCE, Device Code, Client Credentials |
| `a2a-cli/src/token_store.rs` | Token persistence (keyring + AES-256-GCM fallback) |
| `a2a-cli/src/printer.rs` | `print_value` / `print_agent_json` — output formatting and `--fields` filtering |
| `a2a-cli/src/formatter.rs` | `OutputFormat`, table/yaml/csv rendering (sourced from `gws-cli/` via `#[path]`) |
| `a2a-cli/src/error.rs` | `A2aCliError` enum with exit codes |
| `a2a-cli/src/validate.rs` | Input validation helpers |
| `a2a-cli/src/commands/` | Subcommand handlers: `agent`, `auth`, `config`, `schema`, `generate_skills` |

Modules sourced from `gws-cli/` via `#[path]` in `lib.rs`:
- `fs_util` — atomic file writes
- `output` — output formatting primitives
- `credential_store` — AES-256-GCM token encryption and keyring integration
- `formatter` — `OutputFormat` enum + table/yaml/csv rendering (`format_value`)

### Multi-Agent Parallel Execution

Commands resolve targets from (in order of priority):
1. `--agent <alias|url>` (repeatable)
2. `--all` — all registered agents from config
3. `A2A_AGENT_URL` env var
4. Config `current_agent`

With multiple targets, commands dispatch in parallel using `FuturesUnordered` and stream results as NDJSON — first-done-first.

### Auth — Per-Agent

Each registered agent alias has its own OAuth config and token:
- Tokens stored in OS keychain (service: `a2a-cli`) keyed by agent URL hostname:port
- Fallback encrypted files live under `~/.config/a2a-cli/tokens/<host>.enc`
- Backend: `A2A_KEYRING_BACKEND=keyring` (default) or `file` (headless/Docker)
- Flow auto-detected from agent card: AuthCode+PKCE > DeviceCode > ClientCredentials

### A2A Two-Layer Card Protocol

`a2a-cli` implements the full A2A two-layer card protocol:

1. **Public card** (`/.well-known/agent-card.json`) — fetched unauthenticated.
2. **Auth** — OAuth flow triggered from public card's declared schemes.
3. **Extended card** (`/extendedAgentCard`) — fetched after auth when `capabilities.extendedAgentCard: true`.

Both A2A v1.0 and v0.3 agent card formats are handled transparently via `a2a_compat::is_v03()` and `a2a_compat::normalize_card()` in the `a2a-protocol-compat` package.

### A2A SDK

The Rust A2A SDK lives in `a2a-rs/` (git submodule, read-only). Key path dependencies:

| Crate | Purpose |
|-------|---------|
| `a2a-rs/a2a` | Core A2A protocol types (`AgentCard`, `Task`, `Message`, `Part`, etc.) |
| `a2a-rs/a2a-client` | Async A2A client (`A2AClient`, `A2AClientFactory`, `AgentCardResolver`) |
| `a2a-rs/a2acli` | Shared CLI arg structs (`MessageCommand`, `TaskIdCommand`, etc.) |

## Response Shape

`a2a send` wraps the A2A `message/send` operation. The agent decides what to return:

### Task response (most agents)

Output is in `artifacts`. `status.message` is only set for in-progress communication (e.g. `input-required`), not final results.

```json
{
  "id":        "task-abc123",
  "contextId": "ctx-abc123",
  "status": { "state": "completed" },
  "artifacts": [
    {
      "artifactId": "...",
      "parts": [{ "kind": "text", "text": "The agent's answer" }]
    }
  ]
}
```

| `status.state` | Meaning |
|----------------|---------|
| `submitted` | Queued, not started |
| `working` | In progress — poll with `a2a task get <id>` |
| `completed` | Finished — read `artifacts[*].parts` for the answer |
| `failed` | Error — read `status.message` for details |
| `input-required` | Agent needs a reply — read `status.message.parts`, then `a2a send --task-id <id> "..."` |
| `canceled` | Canceled |

### Message response (simple agents)

Some agents return a direct **Message** instead of a Task. The reply is in `parts` at the top level:

```json
{
  "role":  "agent",
  "parts": [{ "kind": "text", "text": "The agent's answer" }]
}
```

Use `--fields .parts` to extract the reply. Multi-agent results include `agent` and `agent_url` at the top level.

## Skills

`skills/a2a/SKILL.md` teaches AI coding tools how to use the `a2a` CLI — commands, flags, response structure, security rules.

Regenerate with:
```bash
a2a generate-skills
```

CI auto-regenerates on push via `.github/workflows/generate-skills.yml`.

## Input Validation

> [!IMPORTANT]
> All CLI argument inputs must be validated before use. See `a2a-cli/src/validate.rs`.
> The validation philosophy and checklist for new features are documented in [`gws-cli/AGENTS.md` — Input Validation & URL Safety](gws-cli/AGENTS.md). Read that section before adding any new flag that accepts user-supplied paths, URLs, or resource identifiers.

`a2a-cli` imports `is_dangerous_unicode` directly from `gws-cli/crates/google-workspace/src/validate.rs` (via `#[path]` in `lib.rs`). The `a2a-cli`-specific validators build on top of it:

| Scenario | Validator | Rejects |
|----------|-----------|---------|
| Agent URL flag | `validate_agent_url()` | Non-http/https, control chars, dangerous Unicode |
| Agent alias | `validate_alias()` | Empty, path separators (`/`, `\`), control chars |
| Message text | `validate_message_text()` | Null bytes, C0/C1 control chars (except `\n`, `\t`), bidi overrides |
| Any flag value | `reject_dangerous_chars()` | All control chars + dangerous Unicode |

Validation is applied:
- `--agent` URL/alias validated in `client::resolve_target` before any network call
- Message text validated in `main::dispatch` before command dispatch

## Testing

Tests are inline `#[cfg(test)]` modules in each source file plus integration tests under `a2a-cli/tests/`. Run with:

```bash
uv run inv test                        # all tests
uv run inv test --filter=validate      # filter by module
```

Current coverage: `error`, `validate`, `printer`, `config`, `auth`, `runner`, plus inherited tests from `credential_store`, `output`, `fs_util`.

## Environment Variables

| Variable | Description |
|----------|-------------|
| `A2A_AGENT_URL` | Default agent alias or URL (single agent) |
| `A2A_BEARER_TOKEN` | Static bearer token — bypasses OAuth for all agents |
| `A2A_KEYRING_BACKEND` | `keyring` (default) or `file` (headless/Docker) |
| `A2A_CLIENT_SECRET` | Client secret for Client Credentials OAuth flow |
| `A2A_CONFIG_DIR` | Override config directory, defaulting to `~/.config/a2a-cli` |
| `A2A_BINARY_PATH` | Override binary path (for npm wrapper) |

## Shell Completions

```bash
# bash — add to ~/.bashrc
source <(a2a completions bash)

# zsh — add to ~/.zshrc
mkdir -p ~/.zsh/completions
a2a completions zsh > ~/.zsh/completions/_a2a
# fpath=(~/.zsh/completions $fpath)
# autoload -Uz compinit && compinit

# fish
a2a completions fish > ~/.config/fish/completions/a2a.fish
```

## Error Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | A2A or HTTP error |
| 2 | Auth error |
| 3 | Invalid input |
| 4 | Config error |
| 5 | Other (I/O, JSON, etc.) |

## Reference Repos

| Directory | Purpose |
|-----------|---------|
| `a2a-rs/` | Rust SDK for A2A protocol (read-only submodule) |
| `gws-cli/` | Google Workspace CLI — source of shared modules and patterns |

These are read-only. Read them for context and patterns; do not modify them.

> [!TIP]
> To understand how a dependency works, read its source in `a2a-rs/` or `gws-cli/` directly. The submodule source is the exact version the project uses — prefer it over web searches.

### gws-cli shared modules

`a2a-cli` directly includes three modules from `gws-cli/` via `#[path]` in `a2a-cli/src/lib.rs`:

| Included from `gws-cli/` | Used for |
|--------------------------|----------|
| `crates/google-workspace-cli/src/fs_util.rs` | Atomic file writes (`atomic_write`) |
| `crates/google-workspace-cli/src/output.rs` | Output formatting, terminal sanitization |
| `crates/google-workspace-cli/src/credential_store.rs` | AES-256-GCM token encryption, keyring integration |
| `crates/google-workspace/src/validate.rs` | `is_dangerous_unicode` (imported in `a2a-cli/src/validate.rs`) |

**Before changing auth, token storage, or validation logic**, read the corresponding implementation in `gws-cli/` — the patterns are intentionally shared. Key reference docs:

- [`gws-cli/AGENTS.md`](gws-cli/AGENTS.md) — build/test conventions, input validation checklist, credential store patterns, PR labels
- [`gws-cli/CONTEXT.md`](gws-cli/CONTEXT.md) — quick reference for the `gws` CLI (different product, useful for understanding shared output and auth patterns)

## PR Labels

- `area: agent` — agent alias registry, config
- `area: auth` — OAuth flows, keychain, token lifecycle
- `area: output` — JSON formatting, field filtering
- `area: skills` — skill generation and management
- `area: validation` — input validation
- `area: npm` — npm packaging, install scripts, platform binaries
- `area: ci` — GitHub Actions workflows
