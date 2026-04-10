# AGENTS.md

## Project Overview

`agc` (Agent CLI) is a Go CLI for interacting with agents that implement the [A2A protocol](https://a2aproject.github.io/A2A/). It is published as `@rover/agent-cli` on npm and as the `agc` binary.

> [!IMPORTANT]
> This CLI is designed to be invoked by AI coding tools (Claude Code, Copilot, Cursor, etc.) as well as humans. Always assume CLI argument inputs can be adversarial — validate paths, reject control characters and dangerous Unicode, and encode user values before embedding in URLs or filenames. See `internal/validate/validate.go`.

> [!NOTE]
> **Reference repos** (`A2A/`, `a2a-go/`, `gws-cli/`) are read-only references. Do not modify them.

## Build & Test

```bash
make build-dev          # Local dev build (auto-computes version)
make test               # go test ./...
make vet                # go vet ./...
make fmt                # go fmt ./...
go build -o bin/agc ./cmd/agc/   # Direct build (no version embedding)
```

Go version: see `go.mod`. The A2A Go SDK (`github.com/a2aproject/a2a-go/v2`) is a published dependency — no local replace directive.

## Architecture

### Commands

| Command | Purpose |
|---------|---------|
| `agc send` | Send a message to one or more agents and wait for response |
| `agc skill` | List or invoke agent skills by skill ID |
| `agc schema` | Inspect A2A protocol types and live agent skill schemas |
| `agc card` | Fetch and display agent card(s) |
| `agc task get/list/cancel` | Task lifecycle management |
| `agc agent add/use/list/remove/show/update` | Named agent alias registry |
| `agc auth login/logout/status` | Per-agent OAuth flows |
| `agc config show/set-timeout/set-client-id/set-scopes` | Config management |
| `agc generate-skills` | Regenerate `skills/` from live agent cards |

### Global Flags

| Flag | Description |
|------|-------------|
| `--agent <alias\|url>` | Target agent (repeatable for parallel multi-agent) |
| `--all` | All registered agents in parallel |
| `--format json\|table\|yaml` | Output format (auto-detects: table in TTY, json when piped) |
| `--fields a,b.c` | Filter output to specific field paths |
| `--transport JSONRPC\|HTTP+JSON` | Override transport (default: auto from agent card) |
| `--timeout 30s` | Request timeout |
| `--bearer-token` | Static bearer token, bypasses OAuth |

### Source Layout

Explore `internal/` for implementation. Key packages:

- `internal/cli/` — all cobra commands; entry point is `internal/cli/root.go`
- `internal/config/` — config file model (`~/.config/agc/config.yaml`)
- `internal/auth/` — OAuth flow selection, token lifecycle, keychain storage
- `internal/output/` — formatting, JSON normalization, terminal sanitization
- `internal/validate/` — input validation helpers

### Multi-Agent Parallel Execution

Commands resolve targets from (in order of priority):
1. `--agent <alias|url>` (repeatable, comma-separated aliases also accepted)
2. `--all` — all registered agents from config
3. `AGC_AGENT_URL` env var
4. Config `current_agent`

With multiple targets, commands dispatch in parallel and stream results NDJSON — first-done-first, each line tagged with `agent` and `agent_url`.

### Output Format

Auto-detects format based on environment: `table` in a terminal, `json` when output is piped. Override with `--format`.

- **table** (default in TTY): human-friendly rendered output
- **json**: normalized `AgentResponse` object (A2A type + extracted text + task metadata)
- **yaml**: YAML representation

The `--fields` flag filters output to specific paths (e.g. `--fields text,task_id`).

### Auth — Per-Agent

Each registered agent alias has its own OAuth config and token:
- Tokens stored in OS keychain (service: `agc`) keyed by agent URL hostname:port
- Fallback: AES-256-GCM encrypted file at `~/.config/agc/tokens/<host>.enc`
- Backend: `AGC_KEYRING_BACKEND=keyring` (default) or `file` (headless/Docker)
- Flow auto-detected from agent card: DeviceCode > AuthCode+PKCE > ClientCredentials > HTTP Bearer > API Key

### A2A Two-Layer Card Protocol

`agc` implements the full A2A two-layer card protocol (spec §13.3):

1. **Public card** (`/.well-known/agent-card.json`) — fetched unauthenticated to bootstrap auth.
2. **Auth** — OAuth flow triggered from public card's declared schemes.
3. **Extended card** (`/extendedAgentCard`) — fetched after auth when `capabilities.extendedAgentCard: true`. Falls back to public card on failure.

The CLI handles both A2A v1.0 and v0.3 agent card formats transparently.

### Skills and Schema

- `agc skill` — lists skills from the agent card; invokes them by sending a message with `skill_id` in metadata.
- `agc schema` — inspects built-in A2A protocol schemas (send, task, message, part, card, skill, artifact) and live agent skill schemas fetched from the card.

Use `agc schema` to understand data structures before constructing `--params` payloads for `agc send`.

## Skills

Skills are SKILL.md files that teach AI coding tools how to use `agc`:

- `skills/agc-shared/SKILL.md` — static reference: agent registration, auth, send, output format
- `skills/agc-agent-<alias>/SKILL.md` — dynamic: generated from live agent card

Regenerate with:
```bash
agc generate-skills               # all registered agents
agc generate-skills prod staging  # specific aliases only
```

CI auto-regenerates on push via `.github/workflows/generate-skills.yml`.

## Input Validation

> [!IMPORTANT]
> All CLI argument inputs must be validated before use. Use the helpers in `internal/validate/validate.go`:

| Scenario | Validator | Rejects |
|----------|-----------|---------|
| Agent URL flag | `validate.AgentURL()` | Non-http/https, null bytes, dangerous Unicode |
| Message text | `validate.MessageText()` | Null bytes, C0/C1 control chars, bidi overrides |
| Task/context IDs | `validate.Identifier()` | `..` traversal, `?#%`, control chars |
| Output dir flag | `validate.SafeOutputDir()` | Absolute paths, traversal, symlinks outside CWD |
| Any flag value | `validate.DangerousInput()` | All control chars + dangerous Unicode |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `AGC_AGENT_URL` | Default agent alias or URL (single agent) |
| `AGC_BEARER_TOKEN` | Static bearer token — bypasses OAuth for all agents |
| `AGC_KEYRING_BACKEND` | `keyring` (default) or `file` (headless/Docker) |
| `AGC_BINARY_PATH` | Override binary path (for npm wrapper) |

## npm Package

Published as `@rover/agent-cli` with optional platform sub-packages:
- `@rover/agent-cli-linux-x64`, `@rover/agent-cli-linux-arm64`
- `@rover/agent-cli-darwin-x64`, `@rover/agent-cli-darwin-arm64`
- `@rover/agent-cli-win32-x64`

```bash
npm install -g @rover/agent-cli   # installs agc globally
npx @rover/agent-cli send "Hello"
```

## Reference Repos

| Directory | Purpose |
|-----------|---------|
| `a2a-go/` | Go SDK for A2A protocol (module: `github.com/a2aproject/a2a-go/v2`) |
| `A2A/` | A2A protocol specification |
| `gws-cli/` | Google Workspace CLI (reference for patterns: skills, auth, output, validation) |

These are read-only. Read them for context and patterns; do not modify them.

> [!TIP]
> To understand how a third-party library works, read its source code directly from the `vendor/` directory (e.g. `vendor/github.com/some/pkg/`). Prefer this over web searches or assumptions — the vendored source is the exact version the project uses.

## PR Labels

- `area: agent` — agent alias registry, config
- `area: auth` — OAuth flows, keychain, token lifecycle
- `area: output` — human rendering, JSON normalization, sanitization
- `area: skills` — skill generation and management
- `area: validation` — input validation
- `area: npm` — npm packaging, install scripts, platform binaries
- `area: ci` — GitHub Actions workflows
