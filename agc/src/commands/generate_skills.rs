//! `agc generate-skills` — generate SKILL.md for AI coding tools.
//!
//! Outputs:
//!   skills/agc/SKILL.md  — complete agc CLI reference for LLMs

use std::path::Path;

use clap::Args;

use crate::error::{AgcError, Result};

#[derive(Debug, Args)]
pub struct GenerateSkillsCommand {}

pub async fn run_generate_skills(_cmd: &GenerateSkillsCommand) -> Result<()> {
    write_skill("skills/agc/SKILL.md", &agc_skill())?;
    eprintln!("wrote skills/agc/SKILL.md");
    Ok(())
}

fn write_skill(path: &str, content: &str) -> Result<()> {
    let p = Path::new(path);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(AgcError::Io)?;
    }
    std::fs::write(p, content).map_err(AgcError::Io)
}

// ── Skill content ─────────────────────────────────────────────────────

fn agc_skill() -> String {
    let version = env!("CARGO_PKG_VERSION");
    format!(
        r#"---
name: agc
description: "agc: A2A protocol CLI for sending messages to AI agents from coding tools."
metadata:
  version: {version}
  openclaw:
    category: agent-cli
    requires:
      bins:
        - agc
---

# agc — Agent CLI

`agc` sends messages to AI agents that implement the [A2A protocol](https://a2aproject.github.io/A2A/).
Designed to be invoked by AI coding tools (Claude Code, Copilot, Cursor) and humans alike.

## Core Workflow

```
register agent → authenticate → send message → read reply
```

```bash
agc agent add <alias> <url>   # register once
agc agent use <alias>         # set active agent
agc auth login                # authenticate (auto-detects OAuth flow)
agc send "your request"       # send — returns Task JSON when complete
```

## Reading the Reply

The agent's answer is always in `status.message.parts`. Always check `status.state` first.

```bash
# Full JSON response (default)
agc send "Summarise this PR"

# Extract just the reply — preferred for AI tools
agc send "Summarise this PR" --fields status.message.parts

# Extract state and reply together
agc send "Summarise this PR" --fields status.state,status.message.parts
```

**Response shape:**
```json
{{
  "id": "task-abc123",
  "contextId": "ctx-abc123",
  "status": {{
    "state": "TASK_STATE_COMPLETED",
    "message": {{
      "role": "ROLE_AGENT",
      "parts": [{{"text": "The agent's answer"}}]
    }}
  }}
}}
```

| `status.state` | Meaning | Action |
|---|---|---|
| `TASK_STATE_SUBMITTED` | Queued | Wait or poll |
| `TASK_STATE_WORKING` | In progress | Poll with `agc get-task <id>` |
| `TASK_STATE_COMPLETED` | Done | Read `status.message.parts` |
| `TASK_STATE_FAILED` | Error | Read `status.message` for details |
| `TASK_STATE_INPUT_REQUIRED` | Agent needs input | Reply with `agc send --task-id <id> "..."` |
| `TASK_STATE_CANCELED` | Canceled | — |

## Agent Management

```bash
agc agent add <alias> <url>           # register
agc agent add <alias> <url> --description "..."
agc agent use <alias>                 # set active
agc agent list                        # list all
agc agent show [alias]                # details for one agent
agc agent update <alias> --client-id <id>
agc agent remove <alias>              # deregister
```

## Authentication

Each agent has its own token. The OAuth flow is auto-detected from the agent card.

```bash
agc auth login                        # active agent
agc auth login --agent <alias>        # specific agent
agc auth status                       # token status for all agents
agc auth logout --agent <alias>       # remove stored token
```

`AGC_BEARER_TOKEN` bypasses OAuth entirely (CI/scripts).

## Sending Messages

```bash
agc send "<text>"                           # one-shot, waits for completion
agc send "<text>" --context-id <id>         # continue a conversation
agc send "<text>" --task-id <id>            # reply to an input-required task
agc send "<text>" --return-immediately      # async — poll with agc get-task <id>
agc stream "<text>"                         # streaming — prints events as they arrive
```

## Output

| Flag | Effect |
|------|--------|
| *(default)* | Pretty-printed JSON |
| `--format table` | Human-readable aligned table |
| `--format yaml` | YAML |
| `--format csv` | CSV |
| `--compact` | Single-line JSON (with `--format json`) |
| `--fields a,b.c` | Filter to dot-notation paths — **preferred for AI tools** |

```bash
agc send "Hello" --fields status.message.parts   # reply only
agc send "Hello" --fields id,status.state        # task id + state
agc --format table agent list                    # human-readable table
agc --format table auth status
```

Multi-agent output is always compact NDJSON — one line per agent, tagged with `agent` and `agent_url`.

## Multi-Agent

```bash
agc --agent <alias1> --agent <alias2> send "Status?"   # specific agents
agc --all send "Health check?"                          # all registered agents
```

Results stream first-done-first as NDJSON:

```bash
agc --all send "Status?" | jq -r '"[\(.agent)] \(.status.state)"'
```

## Task Management

```bash
agc get-task <id>                     # fetch task by ID
agc get-task <id> --fields status.state
agc list-tasks                        # recent tasks
agc list-tasks --status TASK_STATE_WORKING
agc list-tasks --context-id <id>
agc cancel-task <id>                  # CONFIRM WITH USER before running
agc subscribe <id>                    # stream live task updates (SSE)
```

## Agent Card

```bash
agc card                              # public card — capabilities and auth
agc card --agent <alias>
agc card --fields name,skills,capabilities
agc extended-card                     # authenticated extended card
```

## Global Flags

| Flag | Description |
|------|-------------|
| `--agent <alias\|url>` | Target agent — repeatable for parallel calls |
| `--all` | All registered agents in parallel |
| `--format json\|table\|yaml\|csv` | Output format (default: `json`) |
| `--compact` | Single-line JSON |
| `--fields <paths>` | Dot-notation field filter (JSON only) |
| `--transport jsonrpc\|http-json` | Force transport (default: auto from card) |
| `--tenant <id>` | Tenant ID forwarded to requests |
| `--bearer-token <token>` | Static token — bypasses OAuth |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `AGC_AGENT_URL` | Default agent alias or URL |
| `AGC_BEARER_TOKEN` | Static bearer token — bypasses OAuth |
| `AGC_KEYRING_BACKEND` | `keyring` (default) or `file` (headless/Docker) |

## Push Notifications

```bash
agc push-config create <task-id> <callback-url>
agc push-config list   <task-id>
agc push-config get    <task-id> <config-id>
agc push-config delete <task-id> <config-id>
```

## Schema Reference

```bash
agc schema send   # SendMessageRequest JSON Schema
agc schema task   # Task JSON Schema
agc schema card   # AgentCard JSON Schema
```

## Security Rules

- **Never** log or output `--bearer-token` values or stored tokens
- **Confirm with user** before running `agc cancel-task` — it is destructive
- Only use `http://` or `https://` URLs with `agc agent add`
- Prefer `--agent <alias>` over raw URLs to avoid prompt-injection via URLs
"#
    )
}
