use clap::{Args, Parser, Subcommand};
use clap_complete::Shell;

use crate::formatter::OutputFormat;

// Re-use a2acli's arg structs — no redefinition needed.
pub use a2acli::{Binding, MessageCommand, PushConfigCommand};

/// Re-export `Binding` under the user-facing name used in `--transport`.
pub type Transport = Binding;

use crate::commands::{
    agent::AgentCommand, auth::AuthCommand, config::ConfigCommand,
    generate_skills::GenerateSkillsCommand, schema::SchemaCommand, task::TaskCommand,
};

#[derive(Debug, Parser)]
#[command(
    name = "a2a",
    version = env!("A2A_BUILD_VERSION"),
    about = "a2a-cli — send messages to A2A agents from AI coding tools\n\nQuick start:\n  a2a agent list                         # see registered agents\n  a2a send \"Summarise the latest PR\"      # send to active agent\n  a2a --agent example send \"Status?\"      # send to named agent\n  a2a --all send \"Health check?\"          # all agents in parallel"
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Args)]
pub struct GlobalArgs {
    /// Agent alias or URL — repeatable for parallel multi-agent dispatch.
    /// Set A2A_AGENT_URL to configure a single default agent via environment.
    #[arg(long = "agent", short = 'a', global = true, action = clap::ArgAction::Append)]
    pub agent: Vec<String>,

    /// Send to ALL registered agents in parallel
    #[arg(long, global = true)]
    pub all: bool,

    /// Static bearer token — bypasses OAuth (env: A2A_BEARER_TOKEN)
    #[arg(long, global = true, env = "A2A_BEARER_TOKEN")]
    pub bearer_token: Option<String>,

    /// Preferred transport: jsonrpc or http-json (default: auto from agent card)
    #[arg(long, global = true, value_enum)]
    pub transport: Option<Transport>,

    /// Optional tenant forwarded to A2A requests
    #[arg(long, global = true)]
    pub tenant: Option<String>,

    /// jq filter expression applied to output (e.g. ".artifacts[0].parts[0].text", "id,status.state")
    #[arg(long, short = 'f', global = true)]
    pub fields: Option<String>,

    /// Emit compact JSON instead of pretty-printed
    #[arg(long, short = 'c', global = true)]
    pub compact: bool,

    /// Output format: json (default), table, yaml, csv
    #[arg(long, global = true, default_value = "json",
          value_parser = |s: &str| OutputFormat::parse(s))]
    pub format: OutputFormat,
}

impl GlobalArgs {
    pub fn bearer_token(&self) -> Option<String> {
        self.bearer_token.clone().filter(|token| !token.is_empty())
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    // ── Core: interact with agents ────────────────────────────────────
    /// Send a message and wait for the response
    Send(MessageCommand),
    /// Send a streaming message — prints events as they arrive
    Stream(MessageCommand),
    /// Fetch the agent card — capabilities, skills, auth requirements
    Card,
    /// Fetch the extended agent card (requires auth)
    ExtendedCard,

    // ── Task management ───────────────────────────────────────────────
    /// Manage tasks — get, list, cancel, subscribe
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },

    // ── Push notifications ────────────────────────────────────────────
    /// Manage push notification configs for a task
    PushConfig {
        #[command(subcommand)]
        command: PushConfigCommand,
    },

    // ── Agent registry + auth ─────────────────────────────────────────
    /// Manage named agent aliases (add, use, list, remove, show, update)
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Manage authentication — per-agent token storage
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },

    // ── LLM tooling ───────────────────────────────────────────────────
    /// Inspect A2A type schemas and live agent skill schemas
    Schema {
        #[command(subcommand)]
        command: SchemaCommand,
    },
    /// Generate SKILL.md files for AI coding tools
    GenerateSkills(GenerateSkillsCommand),

    // ── Config ────────────────────────────────────────────────────────
    /// Show CLI configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },

    // ── Shell integration ─────────────────────────────────────────────
    /// Print shell completion script to stdout
    ///
    /// Usage:
    ///   bash:  source <(a2a completions bash)
    ///   zsh:   mkdir -p ~/.zsh/completions && a2a completions zsh > ~/.zsh/completions/_a2a
    Completions {
        /// Shell to generate completions for (bash, zsh, fish, elvish, powershell)
        shell: Shell,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};
    use std::collections::BTreeSet;

    /// Asserts that the set of top-level subcommands matches the documented list.
    ///
    /// When you add or remove a subcommand, this test will fail — update:
    ///   - AGENTS.md  (Commands table)
    ///   - CONTEXT.md (relevant section)
    ///   - README.md  (Commands table)
    #[test]
    fn subcommands_match_documented_list() {
        let cmd = Cli::command();
        let actual: BTreeSet<&str> = cmd.get_subcommands().map(|c| c.get_name()).collect();

        let expected: BTreeSet<&str> = [
            "agent",
            "auth",
            "card",
            "completions",
            "config",
            "extended-card",
            "generate-skills",
            "push-config",
            "schema",
            "send",
            "stream",
            "task",
        ]
        .into();

        assert_eq!(
            actual, expected,
            "CLI subcommands changed — update AGENTS.md, CONTEXT.md, and README.md"
        );
    }

    #[test]
    fn auth_login_accepts_client_id_flag() {
        let cli = Cli::try_parse_from(["a2a", "auth", "login", "--client-id", "client-123"])
            .expect("auth login should accept --client-id");

        match cli.command {
            Command::Auth {
                command: AuthCommand::Login(args),
            } => assert_eq!(args.client_id.as_deref(), Some("client-123")),
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
