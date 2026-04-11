---
name: agc-agent-rover-dev
description: "rover-dev agent: Rover is an AI assistant for Toyota/Woven employees. It answers questions using enterprise knowledge bases (Confluence, Jira, Stargate docs) and can perform actions like searching and creating Jira issues."
metadata:
  version: 0.1.0
  openclaw:
    category: agent-cli
    requires:
      bins:
        - agc
      skills:
        - agc-shared
---

# rover-dev — Rover

> Read `../agc-shared/SKILL.md` first for agc CLI flags, auth, and output formatting.

**URL:** https://dev.genai.stargate.toyota/a2a/rover-agent  
**Version:** 1.0.0  

Rover is an AI assistant for Toyota/Woven employees. It answers questions using enterprise knowledge bases (Confluence, Jira, Stargate docs) and can perform actions like searching and creating Jira issues.

## Capabilities

| Feature | Supported |
|---------|-----------|
| Streaming | yes |
| Push notifications | no |
| Extended agent card | no |

## Authentication

This agent requires authentication. Run:

```bash
agc auth login --agent rover-dev
```

Supported schemes: azure-ad

## Skills

### `search_confluence` — Search Confluence

Search and retrieve information from Confluence knowledge base

- **Tags:** search, confluence
**Example messages that trigger this skill:**

```bash
agc --agent rover-dev send "Search Confluence for onboarding guide"
agc --agent rover-dev send "Find the architecture decision records"
```

Inspect this skill's details:
```bash
agc schema skill search_confluence --agent rover-dev
```

### `search_stargate_docs` — Search Stargate Documentation

Search Stargate platform documentation

- **Tags:** search, stargate
**Example messages that trigger this skill:**

```bash
agc --agent rover-dev send "How do I deploy to Stargate?"
agc --agent rover-dev send "What APIs does Stargate provide?"
```

Inspect this skill's details:
```bash
agc schema skill search_stargate_docs --agent rover-dev
```

### `search_personal_files` — Search Personal Files

Search data within files uploaded via Rover

- **Tags:** search, personal-files
**Example messages that trigger this skill:**

```bash
agc --agent rover-dev send "In the files I uploaded, search for my team roadmap this quarter"
agc --agent rover-dev send "Search for my personal goals this year"
```

Inspect this skill's details:
```bash
agc schema skill search_personal_files --agent rover-dev
```

## Examples

```bash
# Send a message (full JSON response)
agc --agent rover-dev send "<your request>"

# Human-readable table output
agc --agent rover-dev --format table send "<your request>"

# AI tools — extract reply parts (A2A spec path)
agc --agent rover-dev send "<your request>" --fields status.message.parts

# Stream the response
agc --agent rover-dev stream "<your request>"

# Check task status after sending
agc --agent rover-dev list-tasks --status working
```

## See Also

- [agc-shared](../agc-shared/SKILL.md) — global flags, auth, output format
