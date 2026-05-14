use std::fmt::Write as FmtWrite;

use clap::{Args, Subcommand};

use crate::cli::GlobalArgs;
use crate::commands::generate_skills::{SkillOutputArgs, display_path, write_skill};
use crate::config::{Agent, OAuthConfig, load, save};
use crate::error::{A2aCliError, Result};
use crate::printer::print_value;
use crate::runner::fetch_card;
use crate::validate::{AgentAlias, validate_agent_url, validate_alias, validate_oauth_client_id};

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
    #[command(flatten)]
    pub output: SkillOutputArgs,

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
            let alias_key = AgentAlias::new(&a.alias)?;
            validate_agent_url(&a.url)?;
            if let Some(client_id) = &a.client_id {
                validate_oauth_client_id(client_id)?;
            }
            let mut cfg = load()?;
            let is_first = cfg.agents.is_empty();
            cfg.agents.insert(
                alias_key.clone(),
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
            if is_first || cfg.current_agent.is_none() {
                cfg.current_agent = Some(alias_key);
                eprintln!("Set {:?} as the active agent.", a.alias);
            }
            save(&cfg)?;
            eprintln!("Agent {:?} → {}", a.alias, a.url);

            // Auto-generate skill from agent card (best-effort — don't fail if unreachable)
            eprint!("fetching card for {} to generate skill... ", a.alias);
            match fetch_card(&a.url, None).await {
                Ok(card) => {
                    let path = std::path::Path::new("skills")
                        .join(&a.alias)
                        .join("SKILL.md");
                    match write_skill(&path, &agent_skill(&a.alias, &a.url, &card)) {
                        Ok(()) => eprintln!("wrote {}", display_path(&path)),
                        Err(e) => eprintln!("skill write failed ({e})"),
                    }
                }
                Err(e) => eprintln!("skipped ({e})"),
            }
        }
        AgentCommand::Use(a) => {
            let alias_key = AgentAlias::new(&a.alias)?;
            let mut cfg = load()?;
            if !cfg.agents.contains_key(alias_key.as_str()) {
                return Err(A2aCliError::Config(format!(
                    "unknown alias {:?} — register with: a2a agent add {} <url>",
                    a.alias, a.alias
                )));
            }
            let url = cfg.agents[alias_key.as_str()].url.clone();
            cfg.current_agent = Some(alias_key);
            save(&cfg)?;
            eprintln!("Active agent: {:?} ({})", a.alias, url);
        }
        AgentCommand::List => {
            let cfg = load()?;
            if cfg.agents.is_empty() {
                eprintln!("No agents registered. Use: a2a agent add <alias> <url>");
                return Ok(());
            }
            let active = cfg.current_agent.as_ref().map(|a| a.as_str()).unwrap_or("");
            let entries: Vec<_> = cfg
                .agents
                .iter()
                .map(|(alias, a)| {
                    serde_json::json!({
                        "alias": alias,
                        "url": a.url,
                        "active": alias.as_str() == active,
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
            validate_alias(&a.alias)?;
            let mut cfg = load()?;
            if !cfg.agents.contains_key(a.alias.as_str()) {
                return Err(A2aCliError::Config(format!("unknown alias {:?}", a.alias)));
            }
            cfg.agents.remove(a.alias.as_str());
            if cfg.current_agent.as_ref().map(|x| x.as_str()) == Some(a.alias.as_str()) {
                cfg.current_agent = None;
                eprintln!(
                    "Removed active agent {:?} — run 'a2a agent use <alias>' to set a new one.",
                    a.alias
                );
            } else {
                eprintln!("Removed agent {:?}.", a.alias);
            }
            save(&cfg)?;
        }
        AgentCommand::Show(a) => {
            let cfg = load()?;
            let current = cfg.current_agent.as_ref().map(|x| x.as_str()).unwrap_or("");
            let alias = a.alias.as_deref().unwrap_or(current);
            if alias.is_empty() {
                return Err(A2aCliError::Config(
                    "no active agent — run: a2a agent use <alias>".to_string(),
                ));
            }
            let agent = cfg
                .agents
                .get(alias as &str)
                .ok_or_else(|| A2aCliError::Config(format!("unknown alias {alias:?}")))?;
            print_value(
                &serde_json::json!({
                    "alias": alias,
                    "url": agent.url,
                    "active": cfg.current_agent.as_ref().map(|x| x.as_str()) == Some(alias),
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
            let output_dir = a.output.resolve_dir()?;
            let aliases: Vec<String> = if a.aliases.is_empty() {
                cfg.agents.keys().map(|k| k.as_str().to_string()).collect()
            } else {
                a.aliases.clone()
            };
            if aliases.is_empty() {
                eprintln!("no agents registered — run: a2a agent add <alias> <url>");
                return Ok(());
            }
            for alias in &aliases {
                validate_alias(alias)?;
                let agent = cfg
                    .agents
                    .get(alias as &str)
                    .ok_or_else(|| A2aCliError::Config(format!("unknown alias {alias:?}")))?;
                eprint!("fetching card for {alias}... ");
                match fetch_card(&agent.url, None).await {
                    Ok(card) => {
                        let path = output_dir.join(alias).join("SKILL.md");
                        write_skill(&path, &agent_skill(alias, &agent.url, &card))?;
                        eprintln!("wrote {}", display_path(&path));
                    }
                    Err(e) => eprintln!("skipped ({e})"),
                }
            }
        }
        AgentCommand::Update(a) => {
            validate_alias(&a.alias)?;
            let mut cfg = load()?;
            let agent = cfg
                .agents
                .get_mut(a.alias.as_str())
                .ok_or_else(|| A2aCliError::Config(format!("unknown alias {:?}", a.alias)))?;
            if let Some(url) = &a.url {
                validate_agent_url(url)?;
                agent.url = url.clone();
            }
            if let Some(desc) = &a.description {
                agent.description = desc.clone();
            }
            if let Some(cid) = &a.client_id {
                validate_oauth_client_id(cid)?;
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

fn agent_skill(alias: &str, url: &str, card: &a2a::AgentCard) -> String {
    let mut s = String::new();
    let version = build_version();
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
    let _ = writeln!(s, "    category: a2a-cli");
    let _ = writeln!(s, "    requires:");
    let _ = writeln!(s, "      bins:");
    let _ = writeln!(s, "        - a2a");
    let _ = writeln!(s, "      skills:");
    let _ = writeln!(s, "        - a2a");
    let _ = writeln!(s, "---");

    let _ = writeln!(s, "\n# {alias} — {agent_name}");
    let _ = writeln!(
        s,
        "\n> Read the `a2a` skill first for CLI flags, auth, and output formatting.\n"
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
            let _ = writeln!(
                s,
                "OAuth login requires a client ID. Save it on the alias or pass it at login time:\n"
            );
            let _ = writeln!(
                s,
                "```bash\na2a agent update {alias} --client-id <id>\na2a auth login --agent {alias}\n\n# or one-off\na2a auth login --agent {alias} --client-id <id>\n```\n"
            );
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
                    let _ = writeln!(s, "a2a send {:?}", ex);
                }
                let _ = writeln!(s, "```");
            } else {
                let _ = writeln!(s, "\n```bash\na2a send \"<describe what you want>\"\n```");
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
    let _ = writeln!(s, "a2a send \"<your request>\"");
    let _ = writeln!(s, "a2a send \"<your request>\" --fields .task.artifacts");
    if caps.streaming.unwrap_or(false) {
        let _ = writeln!(s, "a2a stream \"<your request>\"");
    }
    let _ = writeln!(s, "a2a task list --status working");
    let _ = writeln!(s, "```");

    s
}

fn build_version() -> &'static str {
    option_env!("A2A_BUILD_VERSION").unwrap_or("dev")
}

fn bool_icon(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_utils::EnvGuard;
    use std::path::{Path, PathBuf};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    struct CurrentDirGuard {
        previous: PathBuf,
    }

    impl CurrentDirGuard {
        fn enter(path: &Path) -> Self {
            let previous = std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
            std::env::set_current_dir(path).unwrap();
            Self { previous }
        }
    }

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.previous).unwrap();
        }
    }

    /// Spawn a minimal HTTP server that serves `body` for any GET request.
    async fn spawn_card_server(body: &'static str) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");
        let handle = tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let (read_half, mut write_half) = tokio::io::split(stream);
                let mut reader = BufReader::new(read_half);
                let mut line = String::new();
                // drain request headers
                loop {
                    line.clear();
                    if reader.read_line(&mut line).await.unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                }
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = write_half.write_all(resp.as_bytes()).await;
            }
        });
        (base_url, handle)
    }

    const MINIMAL_CARD: &str = r#"{
        "name": "Test Agent",
        "version": "1.0.0",
        "description": "A test agent",
        "url": "http://127.0.0.1",
        "capabilities": {},
        "defaultInputModes": ["text"],
        "defaultOutputModes": ["text"],
        "skills": []
    }"#;

    fn default_global_args() -> GlobalArgs {
        GlobalArgs {
            agent: vec![],
            all: false,
            fields: None,
            compact: false,
            format: Default::default(),
            transport: None,
            bearer_token: None,
            tenant: None,
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn add_generates_skill_when_card_reachable() {
        let tmp = tempfile::tempdir().unwrap();
        let _cfg_guard = EnvGuard::set("A2A_CONFIG_DIR", tmp.path());
        // Run from the temp dir so skills/<alias>/SKILL.md lands there
        let _dir_guard = CurrentDirGuard::enter(tmp.path());

        let (base_url, _server) = spawn_card_server(MINIMAL_CARD).await;

        let cmd = AgentCommand::Add(AddArgs {
            alias: "myagent".to_string(),
            url: base_url.clone(),
            description: None,
            client_id: None,
            scopes: vec![],
            transport: None,
        });
        run_agent(&cmd, &default_global_args()).await.unwrap();

        let skill_path = tmp.path().join("skills/myagent/SKILL.md");
        assert!(skill_path.exists(), "SKILL.md should be written after add");
        let content = std::fs::read_to_string(&skill_path).unwrap();
        assert!(
            content.contains("myagent"),
            "skill should reference the alias"
        );
        assert!(
            content.contains("Test Agent"),
            "skill should include agent name from card"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn add_succeeds_when_card_unreachable() {
        let tmp = tempfile::tempdir().unwrap();
        let _cfg_guard = EnvGuard::set("A2A_CONFIG_DIR", tmp.path());
        let _dir_guard = CurrentDirGuard::enter(tmp.path());

        // Port 1 is reserved and will be refused immediately
        let cmd = AgentCommand::Add(AddArgs {
            alias: "offline".to_string(),
            url: "http://127.0.0.1:1".to_string(),
            description: None,
            client_id: None,
            scopes: vec![],
            transport: None,
        });
        // add must not fail even if the card fetch fails
        run_agent(&cmd, &default_global_args()).await.unwrap();

        // agent should still be registered
        let cfg = {
            let _cfg_guard2 = EnvGuard::set("A2A_CONFIG_DIR", tmp.path());
            crate::config::load().unwrap()
        };
        assert!(
            cfg.agents.contains_key("offline"),
            "agent should be registered despite card failure"
        );

        // but no skill file should be written
        assert!(!tmp.path().join("skills/offline/SKILL.md").exists());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn generate_skills_respects_output_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let _cfg_guard = EnvGuard::set("A2A_CONFIG_DIR", tmp.path());
        let _dir_guard = CurrentDirGuard::enter(tmp.path());

        let (base_url, _server) = spawn_card_server(MINIMAL_CARD).await;
        let alias = AgentAlias::new("myagent").unwrap();
        let mut cfg = crate::config::Config::default();
        cfg.agents.insert(
            alias,
            Agent {
                url: base_url,
                description: String::new(),
                transport: String::new(),
                oauth: None,
            },
        );
        save(&cfg).unwrap();

        let cmd = AgentCommand::GenerateSkills(GenerateSkillsArgs {
            output: SkillOutputArgs {
                output_dir: Some(".agents/skills".to_string()),
            },
            aliases: vec!["myagent".to_string()],
        });
        run_agent(&cmd, &default_global_args()).await.unwrap();

        assert!(tmp.path().join(".agents/skills/myagent/SKILL.md").exists());
        assert!(!tmp.path().join("skills/myagent/SKILL.md").exists());
    }
}
