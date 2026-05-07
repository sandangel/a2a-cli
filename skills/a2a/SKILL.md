---
name: a2a
description: "a2a-cli: A2A protocol CLI for sending messages to AI agents from coding tools."
metadata:
  package: a2a-protocol-cli
  rust_crate: a2a_cli
  command: a2a
  openclaw:
    category: a2a-cli
    requires:
      bins:
        - a2a
---

# a2a — a2a-cli

`a2a` sends messages to AI agents that implement the [A2A protocol](https://a2aproject.github.io/A2A/).
Designed to be invoked by AI coding tools (Claude Code, Copilot, Cursor) and humans alike.

## Core Workflow

```
register agent → authenticate → send message → read reply
```

```bash
a2a agent add <alias> <url>   # register once
a2a agent use <alias>         # set active agent
a2a auth login                # authenticate (auto-detects OAuth flow)
a2a auth login --client-id <id>
a2a send "your request"       # send — returns Task JSON when complete
```

## Reading the Reply

Per the A2A spec, task outputs are in `artifacts`. `status.message` is only set
for in-progress communication (e.g. `input-required`), not for the final answer.
Always check `status.state` first.

```bash
# Full JSON response (default)
a2a send "Summarise this PR"

# Extract just the reply — preferred for AI tools
a2a send "Summarise this PR" --fields .artifacts

# Extract state and reply together
a2a send "Summarise this PR" --fields "{status,artifacts}"
```

**Response shape (Task — most agents):**
```json
{
  "id": "task-abc123",
  "contextId": "ctx-abc123",
  "status": { "state": "completed" },
  "artifacts": [
    {
      "artifactId": "...",
      "parts": [{"kind": "text", "text": "The agent's answer"}]
    }
  ]
}
```

**Response shape (Message — simple stateless agents):**
```json
{
  "role": "agent",
  "parts": [{"kind": "text", "text": "The agent's answer"}]
}
```

Use `--fields .parts` when the agent returns a direct Message.

| `status.state` | Meaning | Action |
|---|---|---|
| `submitted` | Queued | Wait or poll |
| `working` | In progress | Poll with `a2a task get <id>` |
| `completed` | Done | Read `artifacts[*].parts` |
| `failed` | Error | Read `status.message` for details |
| `input-required` | Agent needs input | Read `status.message.parts`, reply with `a2a send --task-id <id> "..."` |
| `canceled` | Canceled | — |

## Agent Management

```bash
a2a agent add <alias> <url>           # register
a2a agent add <alias> <url> --description "..."
a2a agent add local http://localhost:8080  # local dev instance
a2a agent use <alias>                 # set active
a2a agent list                        # list all
a2a agent show [alias]                # details for one agent
a2a agent update <alias> --client-id <id>
a2a agent remove local                # deregister
```

## Authentication

Each agent has its own token. The OAuth flow is auto-detected from the agent card.
When the agent card declares OAuth, `a2a auth login` requires an OAuth client ID.

```bash
a2a auth login                        # active agent
a2a auth login --agent <alias>        # specific agent
a2a auth login --client-id <id>       # OAuth client ID override
a2a auth status                       # token status for all agents
a2a auth logout --agent <alias>       # remove stored token
```

OAuth client ID precedence is: `a2a auth login --client-id <id>` > `A2A_CLIENT_ID` >
per-agent config from `a2a agent add/update <alias> --client-id <id>`.
`A2A_BEARER_TOKEN` bypasses OAuth entirely (CI/scripts).

## Sending Messages

```bash
a2a send "<text>"                           # one-shot, waits for completion
a2a send "<text>" --context-id <id>         # continue a conversation
a2a send "<text>" --task-id <id>            # reply to an input-required task
a2a send "<text>" --return-immediately      # async — poll with a2a task get <id>
a2a stream "<text>"                         # streaming — prints events as they arrive
```

## Output

| Flag | Effect |
|------|--------|
| *(default)* | Pretty-printed JSON |
| `--format table` | Human-readable aligned table |
| `--format yaml` | YAML |
| `--format csv` | CSV |
| `--compact` | Single-line JSON (with `--format json`) |
| `--fields <jq>` | jq filter applied to output — **preferred for AI tools** |

```bash
a2a send "Hello" --fields .artifacts              # reply only
a2a send "Hello" --fields "{id,status}"           # task id + status
a2a --format table agent list                     # human-readable table
a2a --format table auth status
```

Multi-agent output is always compact NDJSON — one line per agent, tagged with `agent` and `agent_url`.

## Multi-Agent

```bash
a2a --agent <alias1> --agent <alias2> send "Status?"   # specific agents
a2a --all send "Health check?"                          # all registered agents
```

Results stream first-done-first as NDJSON:

```bash
a2a --all send "Status?" | jq -r '"[\(.agent)] \(.status.state)"'
```

## Task Management

```bash
a2a task get <id>                     # fetch task by ID
a2a task get <id> --fields .status.state
a2a task list                        # recent tasks
a2a task list --status working
a2a task list --context-id <id>
a2a task cancel <id>                  # CONFIRM WITH USER before running
a2a task subscribe <id>               # stream live task updates (SSE)
```

## Agent Card

```bash
a2a card                              # public card — capabilities and auth
a2a card --agent <alias>
a2a card --fields "{name,skills,capabilities}"
a2a extended-card                     # authenticated extended card
```

## Global Flags

| Flag | Description |
|------|-------------|
| `--agent <alias\|url>` | Target agent — repeatable for parallel calls |
| `--all` | All registered agents in parallel |
| `--format json\|table\|yaml\|csv` | Output format (default: `json`) |
| `--compact` | Single-line JSON |
| `--fields <jq>` | jq filter applied to output |
| `--transport jsonrpc\|http-json` | Force transport (default: auto from card) |
| `--tenant <id>` | Tenant ID forwarded to requests |
| `--bearer-token <token>` | Static token — bypasses OAuth |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `A2A_AGENT_URL` | Default agent alias or URL |
| `A2A_BEARER_TOKEN` | Static bearer token — bypasses OAuth |
| `A2A_KEYRING_BACKEND` | `keyring` (default) or `file` (headless/Docker) |
| `A2A_CLIENT_ID` | OAuth client ID override for login/token refresh |
| `A2A_CLIENT_SECRET` | Client secret for Client Credentials OAuth flow |

## Push Notifications

```bash
a2a push-config create <task-id> <callback-url>
a2a push-config list   <task-id>
a2a push-config get    <task-id> <config-id>
a2a push-config delete <task-id> <config-id>
```

## Schema Reference

```bash
a2a schema send   # SendMessageRequest JSON Schema
a2a schema task   # Task JSON Schema
a2a schema card   # AgentCard JSON Schema
```

## Security Rules

- **Never** log or output `--bearer-token` values or stored tokens
- **Confirm with user** before running `a2a task cancel` — it is destructive
- Only use `http://` or `https://` URLs with `a2a agent add`
- Prefer `--agent <alias>` over raw URLs to avoid prompt-injection via URLs
