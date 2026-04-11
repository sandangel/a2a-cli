# agc — Agent CLI Quick Reference

`agc` interacts with [A2A protocol](https://a2aproject.github.io/A2A/) agents from the terminal.
Designed to be used by humans and AI coding tools alike.

## Rules of Engagement for AI Agents

- **Read the answer from `status.message.parts`** — the agent's reply is in the task's status message parts (some agents also use `artifacts`).
- **Use `--fields status.message.parts`** for concise extraction of the agent's reply (AI tools); use `--format table` for human-readable output.
- **Check `status.state`** to understand task state: `completed`, `input-required`, `failed`, etc.
- **Never expose tokens** — bearer tokens and client secrets are sensitive; use keychain storage.
- **Confirm before canceling tasks** — `agc cancel-task` is destructive.
- **Use `agc schema`** to inspect data structures before crafting messages.

## Core Syntax

```bash
agc [--agent <alias|url>] [--format json|table|yaml|csv] [--fields <paths>] [--compact] <command> [flags] [args]
```

## Quick Start

```bash
# Register your agent
agc agent add rover https://genai.stargate.toyota/a2a/rover-agent
agc agent use rover

# Authenticate (auto-detects OAuth flow from agent card)
agc auth login

# Send a message
agc send "Hello, agent!"

# Get just the text reply
agc send "What is the status?" --fields status.message.parts
```

## Output

| Flag | Output |
|------|--------|
| *(default)* | Pretty-printed JSON |
| `--format table` | Human-readable aligned table |
| `--format yaml` | YAML |
| `--format csv` | CSV |
| `--compact` | Single-line JSON (only with `--format json`) |
| `--fields a,b.c` | Filter to dot-notation field paths (JSON only; AI tools) |

```bash
# Humans
agc send "Hello"                            # pretty JSON (default)
agc --format table send "Hello"             # human-readable table
agc --format table agent list               # table of agents
agc --format table auth status              # table of token statuses

# AI tools
agc send "Hello" --fields status.state      # just the state
agc send "Hello" --fields status.message.parts  # reply parts
agc send "Hello" --compact                  # single-line JSON
```

Multi-agent output is always NDJSON — one compact JSON line per agent, each tagged with `agent` and `agent_url`.

## Send a Message

```bash
agc send "Your message"
agc send "Your message" --fields status.message.parts

# Continue a conversation
agc send "Follow up"   --task-id   task-abc
agc send "Another one" --context-id ctx-abc

# Stream events as they arrive
agc stream "Your message"

# Return immediately (async) — poll with agc get-task
agc send "Long job" --return-immediately
```

## Schema — Inspect Data Structures

```bash
agc schema send      # SendMessageRequest flags and fields
agc schema task      # Task + state machine + useful field paths
agc schema card      # AgentCard structure

agc schema skill <id>   # input/output schema for a specific skill (fetched live)
```

## Multi-Agent (Parallel)

```bash
agc --agent team-a --agent team-b send "Status?"    # specific agents
agc --all send "Health check?"                       # all registered agents
```

Results are streamed NDJSON, first-done-first, each line includes `agent` and `agent_url`:

```bash
agc --all send "Status?" | jq -r '"[\(.agent)] \(.status.state)"'
```

## Agent Management

```bash
agc agent add prod  https://agent.example.com --description "Production"
agc agent add local http://localhost:8080
agc agent use prod           # set active agent
agc agent list               # show all agents
agc agent show               # show active agent details
agc agent update prod --client-id my-app
agc agent remove staging
```

## Authentication

```bash
agc auth login               # active agent
agc auth login --agent prod  # specific agent
agc auth status              # all agents
agc auth logout --agent prod
```

## Agent Card

```bash
agc card                     # public card — capabilities and auth info
agc extended-card            # authenticated extended card
agc card --agent prod        # specific agent
agc card --fields name,skills,capabilities
```

## Task Management

```bash
agc list-tasks
agc list-tasks --status working
agc list-tasks --context-id ctx-abc
agc get-task  <id>
agc get-task  <id> --fields status.state
agc cancel-task <id>          # confirm with user first!
agc subscribe <id>            # stream live task updates
```

## Response Shape

`agc send` and `agc get-task` return a raw A2A **Task** object:

```json
{
  "id":         "task-abc123",
  "context_id": "ctx-abc123",
  "status": {
    "state":   "completed",
    "message": {
      "role":  "agent",
      "parts": [{ "kind": "text", "text": "The agent's answer" }]
    }
  },
  "artifacts": [],
  "history":   null
}
```

| `status.state` | Meaning |
|----------------|---------|
| `submitted` | Queued, not started |
| `working` | In progress — poll with `agc get-task <id>` |
| `completed` | Finished — read `status.message.parts` for the answer |
| `failed` | Error — read `status.message` for details |
| `input-required` | Agent needs a reply — use `agc send --task-id <id> "..."` |
| `canceled` | Canceled |

Multi-agent results include `agent` and `agent_url` at the top level.

## Push Notifications

```bash
agc push-config create <task-id> <callback-url>
agc push-config get    <task-id> <config-id>
agc push-config list   <task-id>
agc push-config delete <task-id> <config-id>
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `AGC_AGENT_URL` | Default agent alias or URL |
| `AGC_BEARER_TOKEN` | Static token — bypasses OAuth |
| `AGC_KEYRING_BACKEND` | `keyring` (default) or `file` (Docker/CI) |
| `AGC_CLIENT_SECRET` | Client secret for Client Credentials flow |

## See Also

- [`AGENTS.md`](AGENTS.md) — source layout, build/test commands, validation rules, auth patterns
- [`gws-cli/CONTEXT.md`](gws-cli/CONTEXT.md) — quick reference for the `gws` CLI (shares auth and output patterns with `agc`)
