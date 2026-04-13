---
name: rover
description: "Rover is an AI assistant for Toyota/Woven employees. It answers questions using enterprise knowledge bases (Confluence, Jira, Stargate docs) and can perform actions like searching and creating Jira issues."
metadata:
  version: 0.1.0
  openclaw:
    category: agent-cli
    requires:
      bins:
        - agc
      skills:
        - agc
---

# rover — Rover

> Read the `agc` skill first for CLI flags, auth, and output formatting.

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

```bash
agc auth login --agent rover
```

Supported schemes: azure-ad

## Skills

### `search_confluence` — Search Confluence

Search and retrieve information from Confluence knowledge base

- **Tags:** search, confluence

**Example messages:**
```bash
agc send "Search Confluence for onboarding guide"
agc send "Find the architecture decision records"
```

### `search_stargate_docs` — Search Stargate Documentation

Search Stargate platform documentation

- **Tags:** search, stargate

**Example messages:**
```bash
agc send "How do I deploy to Stargate?"
agc send "What APIs does Stargate provide?"
```

### `search_personal_files` — Search Personal Files

Search data within files uploaded via Rover

- **Tags:** search, personal-files

**Example messages:**
```bash
agc send "In the files I uploaded, search for my team roadmap this quarter"
agc send "Search for my personal goals this year"
```

## Quick Reference

```bash
agc send "<your request>"
agc send "<your request>" --fields artifacts
agc stream "<your request>"
agc list-tasks --status working
```
