//! `agc generate-skills` — generate SKILL.md files for AI coding tools.
//!
//! Outputs:
//!   skills/agc-shared/SKILL.md         — static CLI reference (always)
//!   skills/agc-agent-<alias>/SKILL.md  — per-agent from live card

use std::fmt::Write as FmtWrite;
use std::path::Path;

use clap::Args;

use crate::config::load;
use crate::error::{AgcError, Result};
use crate::runner::fetch_card;

#[derive(Debug, Args)]
pub struct GenerateSkillsCommand {
    /// Specific agent aliases to generate for (default: all registered agents)
    pub agents: Vec<String>,
}

pub async fn run_generate_skills(cmd: &GenerateSkillsCommand) -> Result<()> {
    let cfg = load()?;

    // Always regenerate shared skill.
    write_skill("skills/agc-shared/SKILL.md", &shared_skill())?;
    eprintln!("wrote skills/agc-shared/SKILL.md");

    // Determine which agents to process.
    let aliases: Vec<String> = if cmd.agents.is_empty() {
        cfg.agents.keys().cloned().collect()
    } else {
        cmd.agents.clone()
    };

    if aliases.is_empty() {
        eprintln!("no agents registered — run: agc agent add <alias> <url>");
        return Ok(());
    }

    for alias in &aliases {
        let agent = cfg.agents.get(alias).ok_or_else(|| {
            AgcError::Config(format!("unknown alias {alias:?}"))
        })?;

        eprint!("fetching card for {alias}... ");
        match fetch_card(&agent.url, None).await {
            Ok(card) => {
                let path = format!("skills/agc-agent-{alias}/SKILL.md");
                let content = agent_skill(alias, &agent.url, &card);
                write_skill(&path, &content)?;
                eprintln!("wrote {path}");
            }
            Err(e) => eprintln!("skipped ({e})"),
        }
    }

    Ok(())
}

fn write_skill(path: &str, content: &str) -> Result<()> {
    let p = Path::new(path);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(AgcError::Io)?;
    }
    std::fs::write(p, content).map_err(AgcError::Io)
}

// ── agc-shared ────────────────────────────────────────────────────────

fn shared_skill() -> String {
    let version = env!("CARGO_PKG_VERSION");
    format!(
        r#"---
name: agc-shared
description: "agc: CLI for sending messages to A2A protocol agents from AI coding tools."
metadata:
  version: {version}
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
"#
    )
}

// ── agc-agent-<alias> ─────────────────────────────────────────────────

fn agent_skill(alias: &str, url: &str, card: &a2a::AgentCard) -> String {
    let mut s = String::new();
    let version = env!("CARGO_PKG_VERSION");

    let agent_name = &card.name;
    let agent_version = &card.version;
    let description = &card.description;

    // Frontmatter
    let _ = writeln!(
        s,
        r#"---
name: agc-agent-{alias}
description: "{alias} agent: {description}"
metadata:
  version: {version}
  openclaw:
    category: agent-cli
    requires:
      bins:
        - agc
      skills:
        - agc-shared
---"#
    );

    let _ = writeln!(s, "\n# {alias} — {agent_name}");
    let _ = writeln!(
        s,
        "\n> Read `../agc-shared/SKILL.md` first for agc CLI flags, auth, and output formatting.\n"
    );
    let _ = writeln!(s, "**URL:** {url}  ");
    let _ = writeln!(s, "**Version:** {agent_version}  ");
    let _ = writeln!(s, "\n{description}\n");

    // Capabilities
    let _ = writeln!(s, "## Capabilities\n");
    let caps = &card.capabilities;
    let _ = writeln!(
        s,
        "| Feature | Supported |\n|---------|-----------|"
    );
    let _ = writeln!(s, "| Streaming | {} |", bool_icon(caps.streaming.unwrap_or(false)));
    let _ = writeln!(s, "| Push notifications | {} |", bool_icon(caps.push_notifications.unwrap_or(false)));
    let _ = writeln!(s, "| Extended agent card | {} |", bool_icon(caps.extended_agent_card.unwrap_or(false)));

    // Auth
    if let Some(schemes) = &card.security_schemes {
        if !schemes.is_empty() {
            let _ = writeln!(s, "\n## Authentication\n");
            let _ = writeln!(
                s,
                "This agent requires authentication. Run:\n\n```bash\nagc auth login --agent {alias}\n```\n"
            );
            let names: Vec<_> = schemes.keys().cloned().collect();
            let _ = writeln!(s, "Supported schemes: {}", names.join(", "));
        }
    } else {
        let _ = writeln!(s, "\n## Authentication\n\nNo authentication required.");
    }

    // Skills
    if !card.skills.is_empty() {
        let _ = writeln!(s, "\n## Skills\n");
        for skill in &card.skills {
            let _ = writeln!(s, "### `{}` — {}\n", skill.id, skill.name);
            let _ = writeln!(s, "{}
", skill.description);

            // Input/output modes
            if let Some(modes) = &skill.input_modes
                && !modes.is_empty()
            {
                let _ = writeln!(s, "- **Input:** {}", modes.join(", "));
            }
            if let Some(modes) = &skill.output_modes
                && !modes.is_empty()
            {
                let _ = writeln!(s, "- **Output:** {}", modes.join(", "));
            }

            // Tags
            if !skill.tags.is_empty() { let _ = writeln!(s, "- **Tags:** {}", skill.tags.join(", ")); }

            // Skills cannot be invoked by ID — show how to phrase a message for this skill
            if let Some(examples) = &skill.examples
                && !examples.is_empty()
            {
                let _ = writeln!(s, "**Example messages that trigger this skill:**\n");
                let _ = writeln!(s, "```bash");
                for ex in examples {
                    let _ = writeln!(s, "agc --agent {alias} send {:?}", ex);
                }
                let _ = writeln!(s, "```\n");
            } else {
                let _ = writeln!(s, "```bash");
                let _ = writeln!(s, "# Describe what you want — the agent routes to this skill based on message content");
                let _ = writeln!(s, "agc --agent {alias} send \"<describe what you want>\"");
                let _ = writeln!(s, "```\n");
            }

            let _ = writeln!(
                s,
                "Inspect this skill\'s details:\n```bash\nagc schema skill {} --agent {alias}\n```\n",
                skill.id
            );
        }
    } else {
        let _ = writeln!(s, "\n## Skills\n\nThis agent has no declared skills — it accepts general messages.\n");
    }

    // Quick examples
    let _ = writeln!(s, "## Examples\n");
    let _ = writeln!(s, "```bash");
    let _ = writeln!(s, "# Send a message (full JSON response)");
    let _ = writeln!(s, "agc --agent {alias} send \"<your request>\"");
    let _ = writeln!(s);
    let _ = writeln!(s, "# Human-readable table output");
    let _ = writeln!(s, "agc --agent {alias} --format table send \"<your request>\"");
    let _ = writeln!(s);
    let _ = writeln!(s, "# AI tools — extract reply parts (A2A spec path)");
    let _ = writeln!(s, "agc --agent {alias} send \"<your request>\" --fields status.message.parts");

    if caps.streaming.unwrap_or(false) {
        let _ = writeln!(s);
        let _ = writeln!(s, "# Stream the response");
        let _ = writeln!(s, "agc --agent {alias} stream \"<your request>\"");
    }

    let _ = writeln!(s);
    let _ = writeln!(s, "# Check task status after sending");
    let _ = writeln!(s, "agc --agent {alias} list-tasks --status working");
    let _ = writeln!(s, "```");

    let _ = writeln!(s, "\n## See Also\n");
    let _ = writeln!(s, "- [agc-shared](../agc-shared/SKILL.md) — global flags, auth, output format");

    s
}

fn bool_icon(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}
