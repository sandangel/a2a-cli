---
name: agc-shared
description: "agc: CLI for sending messages to A2A protocol agents from AI coding tools."
metadata:
  version: 0.1.0
  openclaw:
    category: agent-cli
    requires:
      bins:
        - agc
---

# agc — Shared Reference

> agc sends messages to AI agents that implement the [A2A protocol](https://a2aproject.github.io/A2A/).
> It is designed to be invoked by AI coding tools (Claude Code, Copilot, Cursor) as well as humans.

## Purpose

Use agc when you need to:
- **Delegate a task** to a specialised AI agent (summarise, generate, analyse, etc.)
- **Query agent state** — check what an agent can do or the status of a running task
- **Coordinate across multiple agents** in parallel

## Quick Start

```bash
# Set up once
agc agent add rover https://genai.stargate.toyota/a2a/rover-agent
agc agent use rover                   # rover is now the active agent

# Send to the active agent — no --agent flag needed
agc send "Summarise the latest PR"
agc send "Status?" --fields status.message.parts

# Target a specific agent or all agents
agc --agent eai send "Status?"        # send to a named agent
agc --all send "Health check?"        # send to all agents in parallel
```

## Agent Registry

Register agents once; refer to them by alias forever.

```bash
agc agent add <alias> <url>           # register
agc agent use <alias>                 # set active
agc agent list                        # list all (shows active with active:true)
agc agent show [alias]                # details for one agent
agc agent remove <alias>              # deregister
```

Registered agents:

| Alias | URL |
|-------|-----|
| `rover` | `https://genai.stargate.toyota/a2a/rover-agent` |
| `eai` | `https://dev.genai.stargate.toyota/a2a/eai-agent` |

```bash
agc agent add rover https://genai.stargate.toyota/a2a/rover-agent
agc agent add eai   https://dev.genai.stargate.toyota/a2a/eai-agent
agc agent use rover   # set rover as the active agent
```

## Authentication

Each agent has its own stored token. The OAuth flow is auto-detected from the agent card.

```bash
agc auth login                        # authenticate active agent
agc auth login --agent rover          # authenticate specific agent
agc auth status                       # token status for all agents
agc auth logout --agent rover         # remove stored token
```

Set `AGC_BEARER_TOKEN` to bypass OAuth entirely (useful in CI/scripts).

## Sending Messages

```bash
agc send "<text>"                                  # one-shot, returns when complete
agc send "<text>" --context-id <id>               # continue a conversation
agc send "<text>" --task-id <id>                  # resume a specific task
agc stream "<text>"                               # streaming — prints events as they arrive
```

## Output

Default output is JSON. Use `--format` to switch to human-readable output; use `--fields` to extract specific paths (AI tools only):

```bash
# Human-readable
agc send "Hello"                                             # pretty JSON (default)
agc --format table send "Hello"                             # human-readable table
agc --format table agent list                               # table of registered agents
agc --format table auth status                              # table of token statuses

# AI tools — extract specific fields from JSON
agc send "Hello" --fields status.message.parts              # reply parts (A2A spec path)
agc send "Hello" --fields id,status.state                   # task ID and state only
agc send "Hello" --compact                                   # single-line JSON
```

The agent's reply is in `status.message.parts` (A2A spec) or `artifacts` (some agents).
Always check `status.state == "completed"` before reading the reply.

Multi-agent output is NDJSON — one JSON object per line, each tagged with `agent` and `agent_url`.

## Multi-Agent

```bash
agc --agent rover --agent eai send "Deploy status?"   # two specific agents
agc --all send "Health check?"                           # all registered agents
```

Results arrive first-done-first as NDJSON lines.

## Task Management

Agents run tasks asynchronously. After `send` returns a task ID:

```bash
agc get-task <id>                     # fetch task by ID
agc list-tasks                        # list recent tasks
agc list-tasks --status working       # filter by state
agc cancel-task <id>                  # cancel a running task
agc subscribe <id>                    # stream live task updates (SSE)
```

Task states: `submitted` → `working` → `completed` | `failed` | `canceled` | `input-required`

## Global Flags

| Flag | Description |
|------|-------------|
| `--agent <alias\|url>` | Target agent — repeatable for parallel calls |
| `--all` | Target all registered agents |
| `--bearer-token <token>` | Static token, bypasses OAuth |
| `--binding jsonrpc\|http-json` | Force transport (default: auto from card) |
| `--tenant <id>` | Tenant ID forwarded to requests |
| `--fields <paths>` | Comma-separated dot-notation field paths to include in output (JSON only) |
| `--format json\|table\|yaml\|csv` | Output format — default `json`; use `table` for human-readable output |
| `--compact` | Single-line JSON (only applies to `--format json`) |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `AGC_AGENT_URL` | Default agent alias or URL |
| `AGC_BEARER_TOKEN` | Static bearer token — bypasses OAuth |
| `AGC_KEYRING_BACKEND` | `keyring` (default) or `file` (headless/Docker) |

## Security Rules

- **Never** log or output bearer tokens
- **Confirm with user** before canceling tasks
- **Validate agent URL** before `agent add` — only http/https accepted
- Prefer `--agent <alias>` over raw URLs to avoid prompt injection via URLs

## Discovering Agent Capabilities

Before sending, inspect what an agent can do:

```bash
agc card                              # fetch active agent's public card
agc card --agent rover                # specific agent
agc schema skill <skill-id>           # understand a skill to craft the right message
agc schema task                       # Task response structure
agc schema send                       # SendMessageRequest structure
```

## See Also

- `skills/agc-agent-<alias>/SKILL.md` — per-agent skills and example commands
