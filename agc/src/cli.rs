use clap::{Args, Parser, Subcommand};

use crate::formatter::OutputFormat;

// Re-use a2acli's arg structs — no redefinition needed.
pub use a2acli::{
    Binding, MessageCommand, PushConfigCommand, TaskIdCommand, TaskLookupCommand,
    ListTasksCommand,
};

/// Re-export `Binding` under the user-facing name used in `--transport`.
pub type Transport = Binding;

use crate::commands::{
    agent::AgentCommand, auth::AuthCommand, config::ConfigCommand,
    generate_skills::GenerateSkillsCommand, schema::SchemaCommand,
};

#[derive(Debug, Parser)]
#[command(
    name = "agc",
    version,
    about = "Agent CLI — send messages to A2A agents from AI coding tools\n\nQuick start:\n  agc agent list                        # see registered agents\n  agc send \"Summarise the latest PR\"     # send to active agent\n  agc --agent prod send \"Status?\"        # send to named agent\n  agc --all send \"Health check?\"         # all agents in parallel"
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Args)]
pub struct GlobalArgs {
    /// Agent alias or URL (env: AGC_AGENT_URL)
    #[arg(long = "agent", short = 'a', global = true, env = "AGC_AGENT_URL")]
    pub agent: Option<String>,

    /// Send to ALL registered agents in parallel
    #[arg(long, global = true)]
    pub all: bool,

    /// Static bearer token — bypasses OAuth (env: AGC_BEARER_TOKEN)
    #[arg(long, global = true, env = "AGC_BEARER_TOKEN")]
    pub bearer_token: Option<String>,

    /// Preferred transport: jsonrpc or http-json (default: auto from agent card)
    #[arg(long, global = true, value_enum)]
    pub transport: Option<Transport>,

    /// Optional tenant forwarded to A2A requests
    #[arg(long, global = true)]
    pub tenant: Option<String>,

    /// Comma-separated field paths to include in output (e.g. "id,status.state")
    #[arg(long, global = true)]
    pub fields: Option<String>,

    /// Emit compact JSON instead of pretty-printed
    #[arg(long, global = true)]
    pub compact: bool,

    /// Output format: json (default), table, yaml, csv
    #[arg(long, global = true, default_value = "json",
          value_parser = |s: &str| OutputFormat::parse(s))]
    pub format: OutputFormat,
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

    /// Fetch a task by ID
    GetTask(TaskLookupCommand),
    /// List tasks with optional filters
    ListTasks(ListTasksCommand),
    /// Cancel a running task
    CancelTask(TaskIdCommand),
    /// Subscribe to live task updates (streaming)
    Subscribe(TaskIdCommand),

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
}
