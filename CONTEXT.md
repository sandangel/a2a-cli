# agc — Agent CLI Quick Reference

`agc` interacts with [A2A protocol](https://a2aproject.github.io/A2A/) agents from the terminal.
Designed to be used by humans and AI coding tools alike.

## Rules of Engagement for AI Agents

- **Read the answer from `.text`** — every response has a `text` field with the agent's plaintext answer.
- **Use `--format json`** for machine-readable output when parsing responses programmatically.
- **Check `type`** to understand response state: `task_completed`, `input_required`, `task_failed`, etc.
- **Never expose tokens** — bearer tokens and client secrets are sensitive; use keychain storage.
- **Confirm before canceling tasks** — `agc task cancel` is destructive.
- **Use `agc schema`** to inspect data structures before constructing `--params` payloads.

## Core Syntax

```bash
agc [--agent <alias>] [--format json] <command> [flags] [args]
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

# Get the answer in scripts / AI tools
agc --format json send "What is the status?" | jq -r .text
```

## Output Modes

| Flag | Output | When to use |
|------|--------|------------|
| *(default)* | Table/text — auto-detects terminal vs pipe | Terminal |
| `--format json` | Normalized `AgentResponse` JSON | Scripts, AI tools |
| `--format yaml` | YAML output | Config-like data |
| `--fields a,b.c` | Filter to specific fields | Extract one value |

Output auto-detects: `table` in a TTY, `json` when stdout is piped.

## Send a Message

```bash
agc send "Your message"
agc --format json send "Your message" | jq -r .text   # extract answer

# Continue a conversation
agc send --task-id   task-abc "Follow up"
agc send --context-id ctx-abc "Another question"

# Full SendMessageRequest via --params (see: agc schema send)
agc send --params '{"message":{"parts":[{"text":"hello"}],"taskId":"task-abc"}}'
```

**Exit codes for `--format json`:** 0 = success, 2 = input_required (reply with `--task-id`), 3 = auth_required (human must authenticate)

## Schema — Inspect Data Structures

```bash
# List all A2A protocol operations
agc schema

# Inspect a built-in type before constructing --params
agc schema send        # SendMessageRequest structure
agc schema task        # Task + state machine
agc schema message     # Message type
agc schema part        # Part (text/data/url/raw)
agc schema card        # AgentCard
agc schema artifact    # Artifact

# Inline all $ref references
agc schema task --resolve-refs
```

## Multi-Agent (Parallel)

```bash
agc --agent team-a --agent team-b send "Status?"    # specific agents
agc --all send "Health check?"                       # all registered agents
agc --format json --all send "Status?" | jq -r '"[\(.agent)] \(.text)"'
```

Results are streamed NDJSON, first-done-first, each line includes `agent` and `agent_url`.

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
agc card                     # active agent capabilities and auth info
agc --format json card       # full card as JSON
agc --all card               # all agents
```

## Task Management

```bash
agc task list
agc task list --status TASK_STATE_WORKING
agc task get  --id task-abc
agc task cancel --id task-abc       # confirm with user first!
agc --all task list                 # across all agents
```

## Response Shape (`--format json`)

```json
{
  "agent":          "prod",
  "agent_url":      "https://agent.example.com",
  "type":           "task_completed",
  "text":           "The agent's answer — always the primary field",
  "task_id":        "task-abc123",
  "context_id":     "ctx-abc123",
  "state":          "TASK_STATE_COMPLETED",
  "artifacts":      [{"id": "...", "name": "report.md", "text": "..."}],
  "input_required": false
}
```

| `type` | Meaning |
|--------|---------|
| `message` | Direct reply |
| `task_completed` | Finished successfully |
| `task_failed` | Failed |
| `task_working` | Still processing — check with `agc task get --id <id>` |
| `input_required` | Agent needs your reply — use `agc send --task-id <id> "..."` |
| `error` | Client-side error (auth, network) |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `AGC_AGENT_URL` | Default agent alias or URL |
| `AGC_BEARER_TOKEN` | Static token — bypasses OAuth |
| `AGC_KEYRING_BACKEND` | `keyring` (default) or `file` (Docker/CI) |
