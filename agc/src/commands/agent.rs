use clap::{Args, Subcommand};

use crate::cli::GlobalArgs;
use crate::config::{Agent, OAuthConfig, load, save};
use crate::error::{AgcError, Result};
use crate::printer::{print_json, print_value};
use crate::validate::{validate_agent_url, validate_alias};

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// Register a named agent alias
    Add(AddArgs),
    /// Set the active agent alias
    Use(AliasArg),
    /// List all registered agent aliases
    List,
    /// Remove a registered agent alias
    #[command(alias = "rm")]
    Remove(AliasArg),
    /// Show details for an agent alias
    Show(OptAliasArg),
    /// Update settings for an existing agent alias
    Update(UpdateArgs),
}

#[derive(Debug, Args)]
pub struct AddArgs {
    pub alias: String,
    pub url: String,
    #[arg(long)]
    pub description: Option<String>,
    #[arg(long)]
    pub client_id: Option<String>,
    #[arg(long, value_delimiter = ',')]
    pub scopes: Vec<String>,
    #[arg(long)]
    pub transport: Option<String>,
}

#[derive(Debug, Args)]
pub struct AliasArg {
    pub alias: String,
}

#[derive(Debug, Args)]
pub struct OptAliasArg {
    pub alias: Option<String>,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    pub alias: String,
    #[arg(long)]
    pub url: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
    #[arg(long)]
    pub client_id: Option<String>,
    #[arg(long, value_delimiter = ',')]
    pub scopes: Vec<String>,
    #[arg(long)]
    pub transport: Option<String>,
}

pub async fn run_agent(cmd: &AgentCommand, args: &GlobalArgs) -> Result<()> {
    match cmd {
        AgentCommand::Add(a) => {
            validate_alias(&a.alias)?;
            validate_agent_url(&a.url)?;
            let mut cfg = load()?;
            let is_first = cfg.agents.is_empty();
            cfg.agents.insert(
                a.alias.clone(),
                Agent {
                    url: a.url.clone(),
                    description: a.description.clone().unwrap_or_default(),
                    transport: a.transport.clone().unwrap_or_default(),
                    oauth: OAuthConfig {
                        client_id: a.client_id.clone().unwrap_or_default(),
                        scopes: a.scopes.clone(),
                    },
                },
            );
            if is_first || cfg.current_agent.is_empty() {
                cfg.current_agent = a.alias.clone();
                eprintln!("Set {:?} as the active agent.", a.alias);
            }
            save(&cfg)?;
            eprintln!("Agent {:?} → {}", a.alias, a.url);
        }
        AgentCommand::Use(a) => {
            validate_alias(&a.alias)?;
            let mut cfg = load()?;
            if !cfg.agents.contains_key(&a.alias) {
                return Err(AgcError::Config(format!(
                    "unknown alias {:?} — register with: agc agent add {} <url>",
                    a.alias, a.alias
                )));
            }
            cfg.current_agent = a.alias.clone();
            save(&cfg)?;
            let url = &cfg.agents[&a.alias].url;
            eprintln!("Active agent: {:?} ({})", a.alias, url);
        }
        AgentCommand::List => {
            let cfg = load()?;
            if cfg.agents.is_empty() {
                eprintln!("No agents registered. Use: agc agent add <alias> <url>");
                return Ok(());
            }
            let entries: Vec<_> = cfg
                .agents
                .iter()
                .map(|(alias, a)| {
                    serde_json::json!({
                        "alias": alias,
                        "url": a.url,
                        "active": alias == &cfg.current_agent,
                        "transport": a.transport,
                        "description": a.description,
                        "client_id": a.oauth.client_id,
                    })
                })
                .collect();
            print_value(&serde_json::Value::Array(entries), args.fields.as_deref(), args.format.clone(), args.compact)?;
        }
        AgentCommand::Remove(a) => {
            let mut cfg = load()?;
            if !cfg.agents.contains_key(&a.alias) {
                return Err(AgcError::Config(format!("unknown alias {:?}", a.alias)));
            }
            cfg.agents.remove(&a.alias);
            if cfg.current_agent == a.alias {
                cfg.current_agent.clear();
                eprintln!("Removed active agent {:?} — run 'agc agent use <alias>' to set a new one.", a.alias);
            } else {
                eprintln!("Removed agent {:?}.", a.alias);
            }
            save(&cfg)?;
        }
        AgentCommand::Show(a) => {
            let cfg = load()?;
            let alias = a.alias.as_deref().unwrap_or(&cfg.current_agent);
            if alias.is_empty() {
                return Err(AgcError::Config(
                    "no active agent — run: agc agent use <alias>".to_string(),
                ));
            }
            let agent = cfg.agents.get(alias).ok_or_else(|| {
                AgcError::Config(format!("unknown alias {alias:?}"))
            })?;
            print_value(
                &serde_json::json!({
                    "alias": alias,
                    "url": agent.url,
                    "active": alias == cfg.current_agent,
                    "transport": agent.transport,
                    "description": agent.description,
                    "oauth": { "client_id": agent.oauth.client_id, "scopes": agent.oauth.scopes },
                }),
                args.fields.as_deref(), args.format.clone(), args.compact,
            )?;
        }
        AgentCommand::Update(a) => {
            let mut cfg = load()?;
            let agent = cfg.agents.get_mut(&a.alias).ok_or_else(|| {
                AgcError::Config(format!("unknown alias {:?}", a.alias))
            })?;
            if let Some(url) = &a.url {
                validate_agent_url(url)?;
                agent.url = url.clone();
            }
            if let Some(desc) = &a.description {
                agent.description = desc.clone();
            }
            if let Some(cid) = &a.client_id {
                agent.oauth.client_id = cid.clone();
            }
            if !a.scopes.is_empty() {
                agent.oauth.scopes = a.scopes.clone();
            }
            if let Some(t) = &a.transport {
                agent.transport = t.clone();
            }
            save(&cfg)?;
            eprintln!("Updated agent {:?}.", a.alias);
        }
    }
    Ok(())
}
