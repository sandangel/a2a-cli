---
name: agc-shared
version: 0.1.0
description: "agc — Agent CLI for A2A protocol agents: setup, auth, sending messages, and task management."
metadata:
  openclaw:
    category: "ai-agents"
    requires:
      bins:
        - agc
    cliHelp: "agc --help"
---

# agc — Shared Reference

`agc` sends messages to [A2A protocol](https://a2aproject.github.io/A2A/) agents and manages the resulting tasks.
Designed for both humans and AI coding tools (Claude Code, Copilot, Cursor, etc.).

## Installation

The `agc` binary must be on `$PATH`.

```bash
npm install -g @rover/agent-cli   # npm (all platforms)
agc --version                     # verify
```

## Setup

```bash
# Register an agent
agc agent add rover https://my-agent.example.com
agc agent use rover

# Authenticate (auto-detects OAuth flow from agent card)
agc auth login

# Verify
agc card
```

## Global Flags

| Flag | Description |
|------|-------------|
| `-a, --agent <alias|url>` | Target agent (repeatable — runs in parallel); env: `AGC_AGENT_URL` |
| `--all` | All registered agents in parallel |
| `-f, --format json|table|yaml` | Output format (default: table in TTY, json when piped) |
| `--fields <paths>` | Comma-separated field paths to filter output (e.g. `text,task_id`) |
| `--timeout <duration>` | Request timeout, e.g. `30s`, `2m` (default: 30s) |
| `--bearer-token <token>` | Static bearer token; env: `AGC_BEARER_TOKEN` — bypasses OAuth |

## Security Rules

- **Never** output bearer tokens or client secrets
- **Always** confirm with the user before canceling tasks (`agc task cancel` is destructive)
- AI agents cannot complete `auth_required` — hand off to a human

## Agent Management

```bash
agc agent add rover https://agent.example.com --description "Production"
agc agent add rover-agent http://localhost:8080
agc agent use rover          # set active agent
agc agent list               # show all agents
agc agent show               # show active agent details
agc agent update rover --client-id my-app
agc agent remove rover
```

## Authentication

Each agent has its own credentials stored securely (OS keychain or encrypted file).

```bash
agc auth login               # active agent — auto-detects OAuth flow from agent card
agc auth login --agent prod  # specific agent
agc auth status              # show token status for all agents
agc auth logout --agent prod
```

## Agent Card

```bash
agc card                     # active agent capabilities and auth info
agc --format json card       # full card as JSON
agc --all card               # all agents
```

## Sending Messages

```bash
# Simple text message
agc send "Your question here"

# JSON output (for scripts / AI tools)
agc --format json send "Your question"

# Continue a task
agc send --task-id task-abc "Follow-up answer"
agc send --context-id ctx-abc "New question in same thread"

# Full request body (see: agc schema send)
agc send --params '{"message":{"parts":[{"text":"hello"}],"taskId":"task-abc"}}'
```

## Reading Responses

`--format json` always outputs raw A2A protocol JSON. Use `agc schema` to learn the structure.

| Command | Output shape | Answer path |
|---|---|---|
| `agc send` | `a2a.Task` or `a2a.Message` | Task: `.status.message.parts[0].text` — Message: `.parts[0].text` |
| `agc task get` | `a2a.Task` | `.status.message.parts[0].text` |
| `agc task cancel` | `a2a.Task` | `.status.message.parts[0].text` |
| `agc task list` | `{agent, agent_url, total_size, tasks: [a2a.Task]}` | per-task: `.status.message.parts[0].text` |
| `agc card` | `a2a.AgentCard` | `agc schema card` |

Detect Task vs Message: Task has `.id` and `.status.state`; Message has `.messageId` and no `.status`.

Task states: `TASK_STATE_COMPLETED` `TASK_STATE_FAILED` `TASK_STATE_CANCELED` `TASK_STATE_REJECTED` `TASK_STATE_WORKING` `TASK_STATE_INPUT_REQUIRED`

## Exit Codes

| Code | Meaning | JSON output |
|------|---------|-------------|
| `0` | Success | Raw `a2a.Task` or `a2a.Message` |
| `1` | Error | Error on stderr |
| `2` | `input_required` | Raw `a2a.Task` — `.id` for task ID, `.status.message.parts[0].text` for the question |
| `3` | `auth_required` | `{type, taskId, contextId, message}` — `.message` has the auth URL/instructions |

## For AI Coding Tools (Claude Code, Copilot, etc.)

```bash
# Use agc schema to learn response structure before parsing
agc schema task     # understand a2a.Task
agc schema message  # understand a2a.Message

result=$(agc --format json send "your request")
exit_code=$?

case $exit_code in
  0)
    # Raw A2A JSON — detect Task vs Message by presence of .status
    text=$(echo "$result" | jq -r '.status.message.parts[0].text // empty')  # Task
    [ -z "$text" ] && text=$(echo "$result" | jq -r '.parts[0].text // empty') # Message
    echo "$text"
    ;;
  2)
    # input_required — raw a2a.Task
    task_id=$(echo "$result" | jq -r '.id')
    question=$(echo "$result" | jq -r '.status.message.parts[0].text // empty')
    echo "Agent asks: $question"
    agc --format json send --task-id "$task_id" "your answer"
    ;;
  3)
    # auth_required — {type, taskId, contextId, message}  (camelCase fields)
    task_id=$(echo "$result" | jq -r '.taskId')
    message=$(echo "$result" | jq -r '.message')
    echo "Human action required: $message"
    echo "  agc --format json task get --id $task_id"
    ;;
esac
```

## Multi-Agent (Parallel)

```bash
# Specific agents in parallel
agc --agent prod --agent staging --format json send "Status?"

# All registered agents
agc --all --format json send "Health check?"
```

Multi-agent send output is NDJSON — one object per line: `{agent, agentUrl, result}` where `result` is the raw `a2a.Task` or `a2a.Message`.

```bash
agc --all --format json send "Status?" | while IFS= read -r line; do
  agent=$(echo "$line" | jq -r .agent)
  text=$(echo "$line" | jq -r '.result.status.message.parts[0].text // .result.parts[0].text // empty')
  echo "[$agent] $text"
done
```

## Task Management

```bash
agc task list
agc task list --status TASK_STATE_WORKING
agc task get --id task-abc
agc task cancel --id task-abc    # confirm with user first!
```

## Schema — Learn Data Structures

Use `agc schema` to inspect A2A protocol types before constructing `--params` payloads.

```bash
agc schema               # overview of all operations
agc schema send          # SendMessageRequest + examples
agc schema task          # Task type + state machine
agc schema message       # Message type
agc schema part          # Part (text/data/url/raw)
agc schema card          # AgentCard
agc schema artifact      # Artifact
agc schema task --resolve-refs   # fully inlined (no $refs)
```

## Config File (`~/.config/agc/config.yaml`)

```yaml
current_agent: rover
agents:
  rover:
    url: https://agent.example.com
    description: "Rover agent"
    oauth:
      client_id: my-app
  local-agent:
    url: http://localhost:8080
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `AGC_AGENT_URL` | Default agent alias or URL |
| `AGC_BEARER_TOKEN` | Static token — bypasses OAuth |
| `AGC_KEYRING_BACKEND` | `keyring` (default) or `file` (Docker/CI) |
