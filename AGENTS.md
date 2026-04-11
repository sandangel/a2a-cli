# AGENTS.md

## Project Overview

`agc` (Agent CLI) is a **Rust** CLI for interacting with agents that implement the [A2A protocol](https://a2aproject.github.io/A2A/). It is published as `@rover/agent-cli` on npm and as the `agc` binary.

> [!IMPORTANT]
> This CLI is designed to be invoked by AI coding tools (Claude Code, Copilot, Cursor, etc.) as well as humans. Always assume CLI argument inputs can be adversarial — validate paths, reject control characters and dangerous Unicode, and encode user values before embedding in URLs or filenames. See `agc/src/validate.rs`.

> [!NOTE]
> **Reference repos** (`A2A/`, `a2a-go/`, `gws-cli/`) are read-only references. Do not modify them.
> `a2a-rs/` is also read-only — it is a git submodule containing the published A2A Rust SDK.

## Build & Test

```bash
cargo build -p agc                        # dev build
cargo build -p agc --release              # release build
cargo test  -p agc                        # run all tests
cargo clippy -p agc -- -D warnings        # lint
cargo fmt   -p agc                        # format

# Output binary: target/debug/agc (dev) or target/release/agc (release)
```

Rust edition: 2024. Minimum Rust version: 1.85. No Makefile — use cargo directly.

## Architecture

### Commands

| Command | Purpose |
|---------|---------|
| `agc send` | Send a message to one or more agents and wait for response |
| `agc stream` | Send a streaming message — prints events as they arrive |
| `agc card` | Fetch and display the public agent card |
| `agc extended-card` | Fetch the authenticated extended agent card |
| `agc get-task <id>` | Fetch a task by ID |
| `agc list-tasks` | List tasks with optional filters |
| `agc cancel-task <id>` | Cancel a running task |
| `agc subscribe <id>` | Subscribe to live task updates (streaming) |
| `agc push-config create/get/list/delete` | Manage push notification configs |
| `agc agent add/use/list/remove/show/update` | Named agent alias registry |
| `agc auth login/logout/status` | Per-agent OAuth flows |
| `agc schema send/task/card/skill <id>` | Inspect A2A protocol types and live skill schemas |
| `agc config show` | Show CLI configuration |
| `agc generate-skills` | Regenerate `skills/` from live agent cards |

### Global Flags

| Flag | Description |
|------|-------------|
| `--agent <alias\|url>` | Target agent (repeatable for parallel multi-agent) |
| `--all` | All registered agents in parallel |
| `--format json\|table\|yaml\|csv` | Output format (default: `json`; use `table` for human-readable) |
| `--compact` | Single-line JSON (only with `--format json`) |
| `--fields a,b.c` | Filter output to dot-notation field paths (`--format json` only; AI tools) |
| `--binding jsonrpc\|http-json` | Override transport (default: auto from agent card) |
| `--tenant <id>` | Optional tenant ID forwarded to A2A requests |
| `--bearer-token <token>` | Static bearer token, bypasses OAuth |

### Source Layout

All implementation is under `agc/src/`. Key modules:

| Module | Purpose |
|--------|---------|
| `agc/src/main.rs` | Entry point — CLI parse, dispatch, multi-agent orchestration |
| `agc/src/cli.rs` | `Cli`, `GlobalArgs`, `Command` enum (clap derive) |
| `agc/src/runner.rs` | `run_to_value` / `run_streaming` — A2A protocol dispatch |
| `agc/src/client.rs` | Agent resolution, `build_http_client`, `resolve_target` |
| `agc/src/config.rs` | Config file model (`~/.config/agc/config.yaml`) |
| `agc/src/auth.rs` | OAuth flow selection, PKCE, Device Code, Client Credentials |
| `agc/src/token_store.rs` | Token persistence (keyring + AES-256-GCM fallback) |
| `agc/src/printer.rs` | `print_value` / `print_agent_json` — output formatting and `--fields` filtering |
| `agc/src/formatter.rs` | `OutputFormat`, table/yaml/csv rendering (sourced from `gws-cli/` via `#[path]`) |
| `agc/src/error.rs` | `AgcError` enum with exit codes |
| `agc/src/validate.rs` | Input validation helpers |
| `agc/src/commands/` | Subcommand handlers: `agent`, `auth`, `config`, `schema`, `generate_skills` |

Modules sourced from `gws-cli/` via `#[path]` in `lib.rs`:
- `fs_util` — atomic file writes
- `output` — output formatting primitives
- `credential_store` — AES-256-GCM token encryption and keyring integration
- `formatter` — `OutputFormat` enum + table/yaml/csv rendering (`format_value`)

### Multi-Agent Parallel Execution

Commands resolve targets from (in order of priority):
1. `--agent <alias|url>` (repeatable)
2. `--all` — all registered agents from config
3. `AGC_AGENT_URL` env var
4. Config `current_agent`

With multiple targets, commands dispatch in parallel using `FuturesUnordered` and stream results NDJSON — first-done-first, each line tagged with `agent` and `agent_url`.

### Output Format

Controlled by `--format` (default: `json`):
- **`json`** (default): pretty-printed JSON; `--compact` makes it single-line
- **`table`**: human-readable aligned table — good for interactive use
- **`yaml`** / **`csv`**: for scripting and data processing

The `--fields` flag pre-filters the JSON value to dot-notation paths before formatting (e.g. `--fields status.state,id`). Applies to all formats.
Multi-agent output (`--all`) is always compact NDJSON regardless of `--format`.

### Auth — Per-Agent

Each registered agent alias has its own OAuth config and token:
- Tokens stored in OS keychain (service: `agc`) keyed by agent URL hostname:port
- Fallback: AES-256-GCM encrypted file at `~/.config/agc/tokens/<host>.enc`
- Backend: `AGC_KEYRING_BACKEND=keyring` (default) or `file` (headless/Docker)
- Flow auto-detected from agent card: AuthCode+PKCE > DeviceCode > ClientCredentials

### A2A Two-Layer Card Protocol

`agc` implements the full A2A two-layer card protocol:

1. **Public card** (`/.well-known/agent-card.json`) — fetched unauthenticated.
2. **Auth** — OAuth flow triggered from public card's declared schemes.
3. **Extended card** (`/extendedAgentCard`) — fetched after auth when `capabilities.extendedAgentCard: true`.

Both A2A v1.0 and v0.3 agent card formats are handled transparently via `a2a_compat::is_v03()` and `a2a_compat::normalize_card()` in the `a2a-compat` crate.

### A2A SDK

The Rust A2A SDK lives in `a2a-rs/` (git submodule, read-only). Key path dependencies:

| Crate | Purpose |
|-------|---------|
| `a2a-rs/a2a` | Core A2A protocol types (`AgentCard`, `Task`, `Message`, `Part`, etc.) |
| `a2a-rs/a2a-client` | Async A2A client (`A2AClient`, `A2AClientFactory`, `AgentCardResolver`) |
| `a2a-rs/a2acli` | Shared CLI arg structs (`MessageCommand`, `TaskIdCommand`, etc.) |

## Skills

Skills are SKILL.md files that teach AI coding tools how to use `agc`:

- `skills/agc-shared/SKILL.md` — static reference: agent registration, auth, send, output format
- `skills/agc-agent-<alias>/SKILL.md` — dynamic: generated from live agent card

The committed lock file `skills-lock.json` tracks the expected skill content checksums.

Regenerate with:
```bash
agc generate-skills               # all registered agents
agc generate-skills prod staging  # specific aliases only
```

CI auto-regenerates on push via `.github/workflows/generate-skills.yml`.

## Input Validation

> [!IMPORTANT]
> All CLI argument inputs must be validated before use. See `agc/src/validate.rs`.
> The validation philosophy and checklist for new features are documented in [`gws-cli/AGENTS.md` — Input Validation & URL Safety](gws-cli/AGENTS.md). Read that section before adding any new flag that accepts user-supplied paths, URLs, or resource identifiers.

`agc` imports `is_dangerous_unicode` directly from `gws-cli/crates/google-workspace/src/validate.rs` (via `#[path]` in `lib.rs`). The `agc`-specific validators build on top of it:

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

Tests are inline `#[cfg(test)]` modules in each source file. Run with:

```bash
cargo test -p agc              # all tests
cargo test -p agc validate     # filter by module
cargo test -p agc -- --nocapture  # show println output
```

Current coverage: `error`, `validate`, `printer`, `config`, `auth`, `runner`, plus inherited tests from `credential_store`, `output`, `fs_util`.

## Environment Variables

| Variable | Description |
|----------|-------------|
| `AGC_AGENT_URL` | Default agent alias or URL (single agent) |
| `AGC_BEARER_TOKEN` | Static bearer token — bypasses OAuth for all agents |
| `AGC_KEYRING_BACKEND` | `keyring` (default) or `file` (headless/Docker) |
| `AGC_BINARY_PATH` | Override binary path (for npm wrapper) |
| `AGC_CLIENT_SECRET` | Client secret for Client Credentials OAuth flow |
| `BUILD_ENV` | `dev` / `stg` / prod (sets default host at compile time) |

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

`agc` directly includes three modules from `gws-cli/` via `#[path]` in `agc/src/lib.rs`:

| Included from `gws-cli/` | Used for |
|--------------------------|----------|
| `crates/google-workspace-cli/src/fs_util.rs` | Atomic file writes (`atomic_write`) |
| `crates/google-workspace-cli/src/output.rs` | Output formatting, terminal sanitization |
| `crates/google-workspace-cli/src/credential_store.rs` | AES-256-GCM token encryption, keyring integration |
| `crates/google-workspace/src/validate.rs` | `is_dangerous_unicode` (imported in `agc/src/validate.rs`) |

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
