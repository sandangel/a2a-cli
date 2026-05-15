//! Resolve an agent alias/URL → ResolvedAgent with validation.

use crate::cli::GlobalArgs;
use crate::config::{Agent, load};
use crate::error::{A2aCliError, Result};
use crate::validate::validate_agent_ref;

pub struct ResolvedAgent {
    pub alias: String,
    pub url: String,
    pub agent: Agent,
}

/// Resolve a single target agent from global args + config.
/// Priority: first --agent / --agents value → A2A_AGENT_URL env → config current_agent.
pub fn resolve_target(args: &GlobalArgs) -> Result<ResolvedAgent> {
    let cfg = load()?;
    let explicit_agents = explicit_agent_refs(args)?;

    let name = if let Some(a) = explicit_agents.first() {
        validate_agent_ref(a)?
    } else if let Ok(env_val) = std::env::var("A2A_AGENT_URL") {
        if !env_val.is_empty() {
            validate_agent_ref(&env_val)?
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
        None => Err(A2aCliError::Config(format!(
            "unknown agent {name:?} — register with: a2a agent add {name} <url>"
        ))),
    }
}

/// Return explicit agent refs from repeated --agent and comma-separated --agents.
pub fn explicit_agent_refs(args: &GlobalArgs) -> Result<Vec<String>> {
    let mut refs = args.agent.clone();

    for raw in &args.agents {
        if raw.trim().is_empty() {
            return Err(A2aCliError::InvalidInput(
                "--agents must not be empty".to_string(),
            ));
        }
        for item in raw.split(',') {
            let item = item.trim();
            if item.is_empty() {
                return Err(A2aCliError::InvalidInput(
                    "--agents must not contain empty entries".to_string(),
                ));
            }
            refs.push(item.to_string());
        }
    }

    Ok(refs)
}

fn resolve_current_agent(cfg: &crate::config::Config) -> Result<String> {
    cfg.current_agent
        .as_ref()
        .map(|a| a.as_str().to_string())
        .ok_or_else(|| {
            A2aCliError::Config(
                "no agent specified — use --agent <alias|url> or run: a2a agent use <alias>"
                    .to_string(),
            )
        })
}

/// Resolve explicit --agent / --agents targets for parallel dispatch.
pub fn resolve_explicit_targets(args: &GlobalArgs) -> Result<Vec<ResolvedAgent>> {
    let cfg = load()?;
    explicit_agent_refs(args)?
        .iter()
        .map(|a| {
            let name = validate_agent_ref(a)?;
            cfg.resolve_agent(&name)
                .map(|agent| {
                    let url = agent.url.clone();
                    ResolvedAgent {
                        alias: name.clone(),
                        url,
                        agent,
                    }
                })
                .ok_or_else(|| {
                    A2aCliError::Config(format!(
                        "unknown agent {name:?} — register with: a2a agent add {name} <url>"
                    ))
                })
        })
        .collect()
}

/// Resolve all target agents for --all.
pub fn resolve_all_targets() -> Result<Vec<ResolvedAgent>> {
    let cfg = load()?;
    if cfg.agents.is_empty() {
        return Err(A2aCliError::Config(
            "no agents registered — use: a2a agent add <alias> <url>".to_string(),
        ));
    }
    Ok(cfg
        .agents
        .into_iter()
        .map(|(alias, agent)| {
            let url = agent.url.clone();
            ResolvedAgent {
                alias: alias.as_str().to_string(),
                url,
                agent,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formatter::OutputFormat;

    fn args(agent: &[&str], agents: &[&str]) -> GlobalArgs {
        GlobalArgs {
            agent: agent.iter().map(|s| (*s).to_string()).collect(),
            agents: agents.iter().map(|s| (*s).to_string()).collect(),
            all: false,
            bearer_token: None,
            transport: None,
            tenant: None,
            fields: None,
            compact: false,
            format: OutputFormat::Json,
        }
    }

    #[test]
    fn explicit_agent_refs_combines_repeated_and_comma_separated_flags() {
        let refs = explicit_agent_refs(&args(&["team-a"], &["team-b,local"])).unwrap();
        assert_eq!(refs, ["team-a", "team-b", "local"]);
    }

    #[test]
    fn explicit_agent_refs_trims_comma_separated_values() {
        let refs = explicit_agent_refs(&args(&[], &["team-a, team-b"])).unwrap();
        assert_eq!(refs, ["team-a", "team-b"]);
    }

    #[test]
    fn explicit_agent_refs_rejects_empty_agents_value() {
        let err = explicit_agent_refs(&args(&[], &[" "])).unwrap_err();
        assert!(matches!(err, A2aCliError::InvalidInput(_)));
    }

    #[test]
    fn explicit_agent_refs_rejects_empty_comma_entry() {
        let err = explicit_agent_refs(&args(&[], &["team-a,,team-b"])).unwrap_err();
        assert!(matches!(err, A2aCliError::InvalidInput(_)));
    }
}
