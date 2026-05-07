# a2a-cli — A2A CLI

`a2a-cli` provides the `a2a` command for interacting with agents that implement the [A2A protocol](https://a2aproject.github.io/A2A/).
It is designed for humans and AI coding tools alike.
The Rust package is `a2a-protocol-cli`, and the library crate is `a2a_cli`.

## Install

### npm (recommended)

GitHub requires authentication to install packages from its registry. The easiest way is using the [GitHub CLI](https://cli.github.com/):

```bash
# 1. Login to GitHub (if not already)
gh auth login --scopes read:packages

# 2. Set the auth token for the npm registry
npm config set //npm.pkg.github.com/:_authToken $(gh auth token)

# 3. Install the CLI
npm install -g @sandangel/a2a-protocol-cli --registry=https://npm.pkg.github.com
```

The GitHub Packages npm package is a binary distribution for the `a2a` command.
Use the Rust crate for programmatic integration.

### Cargo

```bash
cargo install --git https://github.com/sandangel/a2a-cli a2a-protocol-cli
```

### Direct download

Download a pre-built binary from the [Releases](https://github.com/sandangel/a2a-cli/releases) page.

```bash
# Example: Linux x86_64
curl -sLO https://github.com/sandangel/a2a-cli/releases/latest/download/a2a-x86_64-unknown-linux-gnu.tar.gz
tar -xzf a2a-x86_64-unknown-linux-gnu.tar.gz
chmod +x a2a && sudo mv a2a /usr/local/bin/
```

### Build from source

Requires Rust 1.85+.

```bash
git clone https://github.com/sandangel/a2a-cli.git
cd a2a-cli
cargo build -p a2a-protocol-cli --release
# binary: target/release/a2a
```

## Shell completions

```bash
# bash — add to ~/.bashrc
source <(a2a completions bash)

# zsh — add to ~/.zshrc
mkdir -p ~/.zsh/completions
a2a completions zsh > ~/.zsh/completions/_a2a
# then add to ~/.zshrc (if not already present):
#   fpath=(~/.zsh/completions $fpath)
#   autoload -Uz compinit && compinit

# fish
a2a completions fish > ~/.config/fish/completions/a2a.fish
```

## Rust API

Use the Rust crate directly when embedding A2A client behavior in another
Rust application:

```bash
cargo add a2a-protocol-cli --git https://github.com/sandangel/a2a-cli
```

```rust
use a2a_cli::{Client, SendOptions};

#[tokio::main]
async fn main() -> a2a_cli::error::Result<()> {
    let client = Client::new("https://agent.example.com")?;

    let response = client
        .send_with(
            "What is 2+2?",
            SendOptions::default().accept_output("text/plain"),
        )
        .await?;

    println!("{response}");
    Ok(())
}
```

## AI agent skills

Install the `a2a` skill so your AI coding tool (Claude Code, Cursor, Copilot, etc.) knows how to use this CLI:

```bash
# npx
npx skills add sandangel/a2a-cli

# bun
bunx skills add sandangel/a2a-cli
```

To generate per-agent skills from live agent cards:

```bash
a2a agent generate-skills           # all registered agents
a2a agent generate-skills example   # specific alias
```

## Quick start

```bash
# Check existing agents
a2a agent list

# Register an agent
a2a agent add example https://agent.example.com
a2a agent use example

# Authenticate
a2a auth login

# Send a message
a2a send "Hello, agent!"

# Get just the reply (artifacts hold task output per A2A spec)
a2a send "What is 2+2?" --fields .artifacts

# Multi-turn conversation — capture contextId then continue with it
a2a send "My name is San." --fields "{contextId,artifacts}"
a2a send "What is my name?" --context-id <contextId>
```

## Commands

| Command | Description |
|---------|-------------|
| `a2a send` | Send a message and wait for a response |
| `a2a stream` | Send a message and stream events as they arrive |
| `a2a card` | Fetch the public agent card |
| `a2a extended-card` | Fetch the authenticated extended agent card |
| `a2a task get <id>` | Fetch a task by ID |
| `a2a task list` | List tasks with optional filters |
| `a2a task cancel <id>` | Cancel a running task |
| `a2a task subscribe <id>` | Subscribe to live task updates |
| `a2a agent add/use/list/remove/show/update` | Manage named agent aliases |
| `a2a auth login/logout/status` | Per-agent OAuth flows |
| `a2a push-config create/get/list/delete` | Manage push notification configs |
| `a2a schema send/task/card` | Inspect A2A protocol types |
| `a2a config show` | Show CLI configuration |
| `a2a completions <shell>` | Print shell completion script |

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
a2a --format table agent list
a2a --format table auth status

# AI tools — extract just what you need
a2a send "Hello" --fields .artifacts
a2a send "Hello" --compact
```

Multi-agent output is always NDJSON, each line tagged with `agent` and `agent_url`:

```bash
a2a --all send "Status?" | jq -r '"[\(.agent)] \(.status.state)"'
```

## Authentication

Each agent alias has its own OAuth config. Tokens are stored in the OS keychain
under the `a2a-cli` service, keyed by hostname. Use `A2A_KEYRING_BACKEND=file`
for headless / Docker environments.

```bash
a2a auth login               # active agent
a2a auth login --agent example  # specific agent
a2a auth login --client-id <id> # OAuth client ID override
a2a auth status              # all agents
a2a auth logout --agent example
```

OAuth client IDs can be saved per agent with
`a2a agent update <alias> --client-id <id>`, passed to login with
`--client-id`, or supplied through `A2A_CLIENT_ID`.

## Environment variables

| Variable | Description |
|----------|-------------|
| `A2A_AGENT_URL` | Default agent alias or URL |
| `A2A_BEARER_TOKEN` | Static token — bypasses OAuth |
| `A2A_KEYRING_BACKEND` | `keyring` (default) or `file` |
| `A2A_CLIENT_ID` | OAuth client ID override for login/token refresh |
| `A2A_CLIENT_SECRET` | Client secret for Client Credentials flow |
| `A2A_CONFIG_DIR` | Override config directory, defaulting to `~/.config/a2a-cli` |

## Acknowledgements

`a2a-cli` is inspired by and built on patterns from [**gws**](https://github.com/googleworkspace/cli) — the Google Workspace CLI.
Several shared modules (output formatting, credential store, atomic file writes) are included from the `gws-cli` codebase, keeping the two tools consistent in behaviour and structure.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Apache 2.0 — see [LICENSE](LICENSE).
