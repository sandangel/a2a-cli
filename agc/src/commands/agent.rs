use std::fmt::Write as FmtWrite;
use std::path::Path;

use clap::{Args, Subcommand};

use crate::cli::GlobalArgs;
use crate::config::{Agent, OAuthConfig, load, save};
use crate::error::{AgcError, Result};
use crate::printer::print_value;
use crate::runner::fetch_card;
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
    /// Generate SKILL.md for registered agents (default: all)
    GenerateSkills(GenerateSkillsArgs),
}

#[derive(Debug, Args)]
pub struct GenerateSkillsArgs {
    /// Agent aliases to generate for (default: all registered agents)
    pub aliases: Vec<String>,
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
                    oauth: if a.client_id.is_some() || !a.scopes.is_empty() {
                        Some(OAuthConfig {
                            client_id: a.client_id.clone().unwrap_or_default(),
                            scopes: a.scopes.clone(),
                        })
                    } else {
                        None
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
                        "client_id": a.oauth_or_default().client_id,
                    })
                })
                .collect();
            print_value(
                &serde_json::Value::Array(entries),
                args.fields.as_deref(),
                args.format.clone(),
                args.compact,
            )?;
        }
        AgentCommand::Remove(a) => {
            let mut cfg = load()?;
            if !cfg.agents.contains_key(&a.alias) {
                return Err(AgcError::Config(format!("unknown alias {:?}", a.alias)));
            }
            cfg.agents.remove(&a.alias);
            if cfg.current_agent == a.alias {
                cfg.current_agent.clear();
                eprintln!(
                    "Removed active agent {:?} — run 'agc agent use <alias>' to set a new one.",
                    a.alias
                );
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
            let agent = cfg
                .agents
                .get(alias)
                .ok_or_else(|| AgcError::Config(format!("unknown alias {alias:?}")))?;
            print_value(
                &serde_json::json!({
                    "alias": alias,
                    "url": agent.url,
                    "active": alias == cfg.current_agent,
                    "transport": agent.transport,
                    "description": agent.description,
                    "oauth": { "client_id": agent.oauth_or_default().client_id, "scopes": agent.oauth_or_default().scopes },
                }),
                args.fields.as_deref(),
                args.format.clone(),
                args.compact,
            )?;
        }
        AgentCommand::GenerateSkills(a) => {
            let cfg = load()?;
            let aliases: Vec<String> = if a.aliases.is_empty() {
                cfg.agents.keys().cloned().collect()
            } else {
                a.aliases.clone()
            };
            if aliases.is_empty() {
                eprintln!("no agents registered — run: agc agent add <alias> <url>");
                return Ok(());
            }
            for alias in &aliases {
                let agent = cfg
                    .agents
                    .get(alias)
                    .ok_or_else(|| AgcError::Config(format!("unknown alias {alias:?}")))?;
                eprint!("fetching card for {alias}... ");
                match fetch_card(&agent.url, None).await {
                    Ok(card) => {
                        let path = format!("skills/{alias}/SKILL.md");
                        write_skill(&path, &agent_skill(alias, &agent.url, &card))?;
                        eprintln!("wrote {path}");
                    }
                    Err(e) => eprintln!("skipped ({e})"),
                }
            }
        }
        AgentCommand::Update(a) => {
            let mut cfg = load()?;
            let agent = cfg
                .agents
                .get_mut(&a.alias)
                .ok_or_else(|| AgcError::Config(format!("unknown alias {:?}", a.alias)))?;
            if let Some(url) = &a.url {
                validate_agent_url(url)?;
                agent.url = url.clone();
            }
            if let Some(desc) = &a.description {
                agent.description = desc.clone();
            }
            if let Some(cid) = &a.client_id {
                agent
                    .oauth
                    .get_or_insert_with(OAuthConfig::default)
                    .client_id = cid.clone();
            }
            if !a.scopes.is_empty() {
                agent.oauth.get_or_insert_with(OAuthConfig::default).scopes = a.scopes.clone();
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

// ── Agent skill generation ────────────────────────────────────────────

fn write_skill(path: &str, content: &str) -> Result<()> {
    let p = Path::new(path);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(AgcError::Io)?;
    }
    std::fs::write(p, content).map_err(AgcError::Io)
}

fn agent_skill(alias: &str, url: &str, card: &a2a::AgentCard) -> String {
    let mut s = String::new();
    let version = env!("CARGO_PKG_VERSION");
    let agent_name = card.name.trim();
    let agent_version = card.version.trim();
    let description = card.description.trim();
    let caps = &card.capabilities;

    let _ = writeln!(s, "---");
    let _ = writeln!(s, "name: {alias}");
    let _ = writeln!(s, "description: \"{description}\"");
    let _ = writeln!(s, "metadata:");
    let _ = writeln!(s, "  version: {version}");
    let _ = writeln!(s, "  openclaw:");
    let _ = writeln!(s, "    category: agent-cli");
    let _ = writeln!(s, "    requires:");
    let _ = writeln!(s, "      bins:");
    let _ = writeln!(s, "        - agc");
    let _ = writeln!(s, "      skills:");
    let _ = writeln!(s, "        - agc");
    let _ = writeln!(s, "---");

    let _ = writeln!(s, "\n# {alias} — {agent_name}");
    let _ = writeln!(
        s,
        "\n> Read the `agc` skill first for CLI flags, auth, and output formatting.\n"
    );
    let _ = writeln!(s, "**URL:** {url}  ");
    let _ = writeln!(s, "**Version:** {agent_version}  ");
    let _ = writeln!(s, "\n{description}\n");

    // Capabilities
    let _ = writeln!(s, "## Capabilities\n");
    let _ = writeln!(s, "| Feature | Supported |\n|---------|-----------|");
    let _ = writeln!(
        s,
        "| Streaming | {} |",
        bool_icon(caps.streaming.unwrap_or(false))
    );
    let _ = writeln!(
        s,
        "| Push notifications | {} |",
        bool_icon(caps.push_notifications.unwrap_or(false))
    );
    let _ = writeln!(
        s,
        "| Extended agent card | {} |",
        bool_icon(caps.extended_agent_card.unwrap_or(false))
    );

    // Auth
    if let Some(schemes) = &card.security_schemes {
        if !schemes.is_empty() {
            let _ = writeln!(s, "\n## Authentication\n");
            let _ = writeln!(s, "```bash\nagc auth login --agent {alias}\n```\n");
            let _ = writeln!(
                s,
                "Supported schemes: {}",
                schemes.keys().cloned().collect::<Vec<_>>().join(", ")
            );
        }
    } else {
        let _ = writeln!(s, "\n## Authentication\n\nNo authentication required.");
    }

    // Skills
    if !card.skills.is_empty() {
        let _ = writeln!(s, "\n## Skills\n");
        for skill in &card.skills {
            let _ = writeln!(s, "### `{}` — {}\n", skill.id.trim(), skill.name.trim());
            let _ = writeln!(s, "{}\n", skill.description.trim());

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
            if !skill.tags.is_empty() {
                let _ = writeln!(s, "- **Tags:** {}", skill.tags.join(", "));
            }

            if let Some(examples) = &skill.examples
                && !examples.is_empty()
            {
                let _ = writeln!(s, "\n**Example messages:**\n```bash");
                for ex in examples {
                    let _ = writeln!(s, "agc send {:?}", ex);
                }
                let _ = writeln!(s, "```");
            } else {
                let _ = writeln!(s, "\n```bash\nagc send \"<describe what you want>\"\n```");
            }
            let _ = writeln!(s);
        }
    } else {
        let _ = writeln!(
            s,
            "\n## Skills\n\nThis agent has no declared skills — it accepts general messages.\n"
        );
    }

    // Quick reference
    let _ = writeln!(s, "## Quick Reference\n\n```bash");
    let _ = writeln!(s, "agc send \"<your request>\"");
    let _ = writeln!(s, "agc send \"<your request>\" --fields artifacts");
    if caps.streaming.unwrap_or(false) {
        let _ = writeln!(s, "agc stream \"<your request>\"");
    }
    let _ = writeln!(s, "agc task list --status working");
    let _ = writeln!(s, "```");

    s
}

fn bool_icon(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}
