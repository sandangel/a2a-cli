# a2a-cli — A2A CLI

`a2a-cli` provides the `a2a` command for interacting with agents that implement the [A2A protocol](https://a2aproject.github.io/A2A/).
It is designed for humans and AI coding tools alike.

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

The default install uses the stable `latest` npm dist-tag. To install a release
candidate, use the `next` tag. To install the current build from the `main`
branch, use the `dev` tag:

```bash
# Release candidate
npm install -g @sandangel/a2a-protocol-cli@next --registry=https://npm.pkg.github.com

# Current main-branch build
npm install -g @sandangel/a2a-protocol-cli@dev --registry=https://npm.pkg.github.com
```

The GitHub Packages npm package is a binary distribution for the `a2a` command.
For Cargo installs, direct binary downloads, source builds, shell completions,
and Rust library usage, see [INSTALL.md](INSTALL.md).

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
a2a send "What is 2+2?" --fields .task.artifacts

# Multi-turn conversation — capture contextId then continue with it
a2a send "My name is Harry Potter." --fields .task.contextId
a2a send "What is my name?" --context-id <contextId>
```

## AI agent skills

Install the `a2a` skill so your AI coding tool (Claude Code, Cursor, Copilot, etc.) knows how to use this CLI:

```bash
# with a2a itself
a2a generate-skills --output-dir .agents/skills

# npx
npx skills add sandangel/a2a-cli

# bun
bunx skills add sandangel/a2a-cli
```

To generate per-agent skills from live agent cards:

```bash
a2a agent generate-skills           # all registered agents
a2a agent generate-skills example   # specific alias
a2a agent generate-skills --output-dir .agents/skills
```

Both skill generators accept `--output-dir <DIR>`. The directory must be a
relative path under the current project. Defaults:

| Command                             | Default output            |
| ----------------------------------- | ------------------------- |
| `a2a generate-skills`               | `skills/a2a/SKILL.md`     |
| `a2a agent generate-skills example` | `skills/example/SKILL.md` |

## Commands

| Command                                                   | Description                                     |
| --------------------------------------------------------- | ----------------------------------------------- |
| `a2a send`                                                | Send a message and wait for a response          |
| `a2a stream`                                              | Send a message and stream events as they arrive |
| `a2a card`                                                | Fetch the public agent card                     |
| `a2a extended-card`                                       | Fetch the authenticated extended agent card     |
| `a2a task get <id>`                                       | Fetch a task by ID                              |
| `a2a task list`                                           | List tasks with optional filters                |
| `a2a task cancel <id>`                                    | Cancel a running task                           |
| `a2a task subscribe <id>`                                 | Subscribe to live task updates                  |
| `a2a agent add/use/list/remove/show/update`               | Manage named agent aliases                      |
| `a2a agent generate-skills [--output-dir DIR] [alias...]` | Generate per-agent skills from live agent cards |
| `a2a auth login/logout/status`                            | Per-agent OAuth flows                           |
| `a2a push-config create/get/list/delete`                  | Manage push notification configs                |
| `a2a schema send/task/card`                               | Inspect A2A protocol types and discover shapes  |
| `a2a config show`                                         | Show CLI configuration                          |
| `a2a generate-skills [--output-dir DIR]`                  | Generate the `a2a` CLI skill                    |
| `a2a completions <shell>`                                 | Print shell completion script                   |

## Global flags

| Flag                              | Description                                        |
| --------------------------------- | -------------------------------------------------- |
| `--agent <alias\|url>`            | Target agent (repeatable for multi-agent)          |
| `--agents <alias[,alias...]>`     | Comma-separated target agents for multi-agent      |
| `--all`                           | All registered agents in parallel                  |
| `--format json\|table\|yaml\|csv` | Output format (default: `json`)                    |
| `--compact`                       | Single-line JSON                                   |
| `--fields <jq>`                   | Built-in jq filter applied to output (e.g. `.task.artifacts[0]`) |
| `--bearer-token <token>`          | Static API token / bearer token, bypasses OAuth    |

## Output

`a2a send` returns `SendMessageResponse` JSON for all supported server
versions. v0.3 server responses are normalized by the compatibility layer, so
task replies are always under `.task.*`.

Use `--fields` to run the CLI's built-in jq filter against the JSON output. Use
`a2a schema` to discover request and response shapes before writing filters or
constructing payloads:

```bash
a2a schema send   # SendMessageRequest JSON Schema
a2a schema task   # Task JSON Schema
a2a schema card   # AgentCard JSON Schema
```

```bash
# Human-readable
a2a --format table agent list
a2a --format table auth status

# AI tools — extract just what you need
a2a send "Hello" --fields .task.artifacts
a2a send "Hello" --compact
```

Multi-agent output is always NDJSON, each line tagged with `agent` and `agent_url`:

```bash
a2a --agents <alias1>,<alias2> send "Status?" --fields "{agent,state:.task.status.state}"
a2a --all send "Status?" --fields "{agent,state:.task.status.state}"
```

## Authentication

`a2a` supports OAuth, static API tokens, and unauthenticated agents. Each agent
alias has its own OAuth config. Tokens are stored in the OS keychain under the
`a2a-cli` service, keyed by hostname. Use `A2A_KEYRING_BACKEND=file` for
headless / Docker environments.

`a2a auth login` auto-detects the first supported OAuth flow declared by the
agent card. Supported auth modes:

| Auth mode | How to use it | Notes |
|-----------|---------------|-------|
| No auth | No token or OAuth config required | Used when the agent card declares no OAuth flow and no bearer token is supplied |
| Bearer / API token | `A2A_BEARER_TOKEN=<token>` or `--bearer-token <token>` | Bypasses OAuth and sends `Authorization: Bearer <token>` |
| OAuth `authorizationCode` + PKCE | `a2a auth login --client-id <id>` | Opens a browser and listens on a local callback URL; refreshes later if the token response includes `refresh_token` |
| OAuth `deviceCode` | `a2a auth login --client-id <id>` | Prints the verification URL and user code for browser/device login; refreshes later if the token response includes `refresh_token` |
| OAuth `clientCredentials` | `A2A_CLIENT_ID=<id> A2A_CLIENT_SECRET=<secret> a2a auth login` | Uses the token endpoint directly; renews expired tokens with `A2A_CLIENT_SECRET` |

OAuth `implicit` and password grants are not implemented.

```bash
a2a agent update example --client-id <client-id>
a2a auth login --agent example
a2a --agent example send "Hello, agent!"
```

CIMD deployments are also supported. In that setup, the OAuth client ID is an
HTTP URL; pass that URL unchanged as the client ID.

```bash
a2a auth login --agent example --client-id http://cimd.example.com/clients/a2a-cli
```

For an API token / bearer token, use `A2A_BEARER_TOKEN` in scripts or CI/CD, or
pass `--bearer-token` for a one-off command.

```bash
export A2A_BEARER_TOKEN=<api-token>
a2a --agent example send "Hello, agent!"

a2a --agent example --bearer-token <api-token> send "Hello, agent!"
```

For OAuth Client Credentials, provide the client ID and secret. The agent card
must declare the Client Credentials flow.

```bash
export A2A_CLIENT_ID=<client-id>
export A2A_CLIENT_SECRET=<client-secret>
a2a auth login --agent example
a2a --agent example send "Hello, agent!"
```

Common auth commands:

```bash
a2a auth login                    # active agent
a2a auth login --agent example    # specific agent
a2a auth login --client-id <id>   # OAuth client ID override
a2a auth status                   # all agents
a2a auth logout --agent example
```

OAuth client IDs can be saved per agent with
`a2a agent update <alias> --client-id <id>`, passed to login with
`--client-id`, or supplied through `A2A_CLIENT_ID`.
Agent-facing commands use the stored token and renew expired tokens automatically
when possible. Client Credentials renewal requires `A2A_CLIENT_SECRET`.

## Environment variables

| Variable              | Description                                                  |
| --------------------- | ------------------------------------------------------------ |
| `A2A_AGENT_URL`       | Default agent alias or URL                                   |
| `A2A_BEARER_TOKEN`    | Static token — bypasses OAuth                                |
| `A2A_KEYRING_BACKEND` | `keyring` (default) or `file`                                |
| `A2A_CLIENT_ID`       | OAuth client ID override for login/token refresh             |
| `A2A_CLIENT_SECRET`   | Client secret for Client Credentials flow                    |
| `A2A_CONFIG_DIR`      | Override config directory, defaulting to `~/.config/a2a-cli` |

## Acknowledgements

`a2a-cli` is inspired by and built on patterns from [**gws**](https://github.com/googleworkspace/cli) — the Google Workspace CLI.
Several shared modules (output formatting, credential store, atomic file writes) are included from the `gws-cli` codebase, keeping the two tools consistent in behaviour and structure.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Apache 2.0 — see [LICENSE](LICENSE).
