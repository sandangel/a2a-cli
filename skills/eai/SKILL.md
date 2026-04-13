---
name: eai
description: "EAI assistant answers questions using EAI documentation and resources."
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

# eai — EAI

> Read the `agc` skill first for CLI flags, auth, and output formatting.

**URL:** https://dev.genai.stargate.toyota/a2a/eai-agent  
**Version:** 1.0.0  

EAI assistant answers questions using EAI documentation and resources.

## Capabilities

| Feature | Supported |
|---------|-----------|
| Streaming | yes |
| Push notifications | no |
| Extended agent card | no |

## Authentication

```bash
agc auth login --agent eai
```

Supported schemes: azure-ad

## Skills

### `search_eai_docs` — Search EAI Documentation

Search EAI documentation and resources

- **Tags:** search, eai

**Example messages:**
```bash
agc send "How do I get started with EAI?"
agc send "What are the EAI APIs?"
```

## Quick Reference

```bash
agc send "<your request>"
agc send "<your request>" --fields artifacts
agc stream "<your request>"
agc list-tasks --status working
```
