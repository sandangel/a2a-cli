# agc — Agent CLI

`agc` is a CLI for interacting with agents that implement the [A2A protocol](https://a2aproject.github.io/A2A/).
Designed to be used by humans and AI coding tools alike.

## Install

### npm (recommended)

```bash
npm install -g @rover/agent-cli --registry https://artifactory.stargate.toyota/artifactory/api/npm/rover-npm-release/
```

### Direct download

Download a pre-built binary from the [Releases](https://github.com/sg-genai/genai-cli/releases) page.

```bash
# Example: Linux x86_64
curl -sLO https://github.com/sg-genai/genai-cli/releases/latest/download/agc-x86_64-unknown-linux-gnu.tar.gz
tar -xzf agc-x86_64-unknown-linux-gnu.tar.gz
chmod +x agc && sudo mv agc /usr/local/bin/
```

### Build from source

Requires Rust 1.85+.

```bash
git clone https://github.com/sg-genai/genai-cli.git
cd genai-cli
cargo build -p agc --release
# binary: target/release/agc
```

## Shell completions

```bash
# bash — add to ~/.bashrc
source <(agc completions bash)

# zsh — add to ~/.zshrc
mkdir -p ~/.zsh/completions
agc completions zsh > ~/.zsh/completions/_agc
# then add to ~/.zshrc (if not already present):
#   fpath=(~/.zsh/completions $fpath)
#   autoload -Uz compinit && compinit

# fish
agc completions fish > ~/.config/fish/completions/agc.fish
```

## AI agent skills

Install the `agc` skill so your AI coding tool (Claude Code, Cursor, Copilot, etc.) knows how to use this CLI:

```bash
# npx
npx skills add sg-genai/genai-cli

# bun
bunx skills add sg-genai/genai-cli
```

To generate per-agent skills from live agent cards:

```bash
agc agent generate-skills           # all registered agents
agc agent generate-skills rover     # specific alias
```

## Quick start

```bash
# Register an agent
agc agent add rover https://genai.stargate.toyota/a2a/rover-agent
agc agent use rover

# Authenticate
agc auth login

# Send a message
agc send "Hello, agent!"

# Get just the reply (artifacts hold task output per A2A spec)
agc send "What is 2+2?" --fields .artifacts

# Multi-turn conversation — capture contextId then continue with it
agc send "My name is San." --fields "{contextId,artifacts}"
agc send "What is my name?" --context-id <contextId>
```

## Commands

| Command | Description |
|---------|-------------|
| `agc send` | Send a message and wait for a response |
| `agc stream` | Send a message and stream events as they arrive |
| `agc card` | Fetch the public agent card |
| `agc extended-card` | Fetch the authenticated extended agent card |
| `agc task get <id>` | Fetch a task by ID |
| `agc task list` | List tasks with optional filters |
| `agc task cancel <id>` | Cancel a running task |
| `agc task subscribe <id>` | Subscribe to live task updates |
| `agc agent add/use/list/remove/show/update` | Manage named agent aliases |
| `agc auth login/logout/status` | Per-agent OAuth flows |
| `agc push-config create/get/list/delete` | Manage push notification configs |
| `agc schema send/task/card` | Inspect A2A protocol types |
| `agc config show` | Show CLI configuration |
| `agc completions <shell>` | Print shell completion script |

## Global flags

| Flag | Description |
|------|-------------|
| `--agent <alias\|url>` | Target agent (repeatable for multi-agent) |
| `--all` | All registered agents in parallel |
| `--format json\|table\|yaml\|csv` | Output format (default: `json`) |
| `--compact` | Single-line JSON |
| `--fields <jq>` | jq filter applied to output (e.g. `.artifacts[0]`) |

## Output

```bash
# Human-readable
agc --format table agent list
agc --format table auth status

# AI tools — extract just what you need
agc send "Hello" --fields .status.message.parts
agc send "Hello" --compact
```

Multi-agent output is always NDJSON, each line tagged with `agent` and `agent_url`:

```bash
agc --all send "Status?" | jq -r '"[\(.agent)] \(.status.state)"'
```

## Authentication

Each agent alias has its own OAuth config. Tokens are stored in the OS keychain
(`agc` service, keyed by hostname). Use `AGC_KEYRING_BACKEND=file` for
headless / Docker environments.

```bash
agc auth login               # active agent
agc auth login --agent rover  # specific agent
agc auth status              # all agents
agc auth logout --agent rover
```

## Environment variables

| Variable | Description |
|----------|-------------|
| `AGC_AGENT_URL` | Default agent alias or URL |
| `AGC_BEARER_TOKEN` | Static token — bypasses OAuth |
| `AGC_KEYRING_BACKEND` | `keyring` (default) or `file` |
| `AGC_CLIENT_SECRET` | Client secret for Client Credentials flow |

## Acknowledgements

`agc` is inspired by and built on patterns from [**gws**](https://github.com/googleworkspace/cli) — the Google Workspace CLI.
Several internal modules (output formatting, credential store, atomic file writes) are shared directly from the `gws-cli` codebase, keeping the two tools consistent in behaviour and structure.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Apache 2.0 — see [LICENSE](LICENSE).
