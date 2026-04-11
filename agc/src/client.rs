//! Resolve an agent alias/URL → ResolvedAgent with validation.

use crate::cli::GlobalArgs;
use crate::config::{Agent, load};
use crate::error::{AgcError, Result};
use crate::validate::{validate_agent_url, validate_alias};

pub struct ResolvedAgent {
    pub alias: String,
    pub url: String,
    pub agent: Agent,
}

/// Resolve the target agent from global args + config.
/// Returns the alias (or URL), the agent config, and the bearer token if any.
pub fn resolve_target(args: &GlobalArgs) -> Result<ResolvedAgent> {
    let cfg = load()?;

    let name = match args.agent.as_deref() {
        Some(a) => {
            // Validate user-supplied --agent value before use.
            if a.starts_with("http://") || a.starts_with("https://") {
                validate_agent_url(a)?;
            } else {
                validate_alias(a)?;
            }
            a.to_string()
        }
        None => {
            if cfg.current_agent.is_empty() {
                return Err(AgcError::Config(
                    "no agent specified — use --agent <alias|url> or run: agc agent use <alias>"
                        .to_string(),
                ));
            }
            cfg.current_agent.clone()
        }
    };

    match cfg.resolve_agent(&name) {
        Some(agent) => Ok(ResolvedAgent {
            alias: name.clone(),
            url: agent.url.clone(),
            agent,
        }),
        None => Err(AgcError::Config(format!(
            "unknown agent {name:?} — register with: agc agent add {name} <url>"
        ))),
    }
}

/// Resolve all target agents for --all.
pub fn resolve_all_targets() -> Result<Vec<ResolvedAgent>> {
    let cfg = load()?;
    if cfg.agents.is_empty() {
        return Err(AgcError::Config(
            "no agents registered — use: agc agent add <alias> <url>".to_string(),
        ));
    }
    Ok(cfg
        .agents
        .into_iter()
        .map(|(alias, agent)| {
            let url = agent.url.clone();
            ResolvedAgent { alias, url, agent }
        })
        .collect())
}
