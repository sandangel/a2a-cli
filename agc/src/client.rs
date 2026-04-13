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

/// Resolve a single target agent from global args + config.
/// Priority: first --agent value → AGC_AGENT_URL env → config current_agent.
pub fn resolve_target(args: &GlobalArgs) -> Result<ResolvedAgent> {
    let cfg = load()?;

    let name = if let Some(a) = args.agent.first() {
        // Validate user-supplied --agent value before use.
        if a.starts_with("http://") || a.starts_with("https://") {
            validate_agent_url(a)?;
        } else {
            validate_alias(a)?;
        }
        a.clone()
    } else if let Ok(env_val) = std::env::var("AGC_AGENT_URL") {
        if !env_val.is_empty() {
            if env_val.starts_with("http://") || env_val.starts_with("https://") {
                validate_agent_url(&env_val)?;
            } else {
                validate_alias(&env_val)?;
            }
            env_val
        } else {
            resolve_current_agent(&cfg)?
        }
    } else {
        resolve_current_agent(&cfg)?
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

fn resolve_current_agent(cfg: &crate::config::Config) -> Result<String> {
    if cfg.current_agent.is_empty() {
        return Err(AgcError::Config(
            "no agent specified — use --agent <alias|url> or run: agc agent use <alias>"
                .to_string(),
        ));
    }
    Ok(cfg.current_agent.clone())
}

/// Resolve explicit --agent targets (for `--agent a --agent b` parallel dispatch).
pub fn resolve_explicit_targets(args: &GlobalArgs) -> Result<Vec<ResolvedAgent>> {
    let cfg = load()?;
    args.agent
        .iter()
        .map(|a| {
            if a.starts_with("http://") || a.starts_with("https://") {
                validate_agent_url(a)?;
            } else {
                validate_alias(a)?;
            }
            cfg.resolve_agent(a)
                .map(|agent| {
                    let url = agent.url.clone();
                    ResolvedAgent {
                        alias: a.clone(),
                        url,
                        agent,
                    }
                })
                .ok_or_else(|| {
                    AgcError::Config(format!(
                        "unknown agent {a:?} — register with: agc agent add {a} <url>"
                    ))
                })
        })
        .collect()
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
