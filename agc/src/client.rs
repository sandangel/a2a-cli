//! Resolve an agent alias/URL → A2AClient with auth applied.

use std::sync::Arc;

use a2a::AgentCard;
use a2a_client::{A2AClient, A2AClientFactory, agent_card::AgentCardResolver, auth::AuthInterceptor};

use crate::cli::GlobalArgs;
use crate::config::{Agent, load};
use crate::error::{AgcError, Result};

pub struct ResolvedAgent {
    pub alias: String,
    pub url: String,
    pub agent: Agent,
}

/// Resolve the target agent from global args + config.
/// Returns the alias (or URL), the agent config, and the bearer token if any.
pub fn resolve_target(args: &GlobalArgs) -> Result<ResolvedAgent> {
    // --bearer-token / env bypass: no config needed if URL given directly.
    let name = match args.agent.as_deref() {
        Some(a) => a.to_string(),
        None => {
            // Fall back to config current_agent.
            let cfg = load()?;
            if cfg.current_agent.is_empty() {
                return Err(AgcError::Config(
                    "no agent specified — use --agent <alias|url> or run: agc agent use <alias>"
                        .to_string(),
                ));
            }
            cfg.current_agent.clone()
        }
    };

    let cfg = load()?;
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

/// Fetch agent card for a URL.
pub async fn fetch_card(url: &str, bearer: Option<&str>) -> Result<AgentCard> {
    let http = build_http_client(bearer)?;
    let resolver = AgentCardResolver::new(Some(http));
    resolver.resolve(url).await.map_err(AgcError::A2A)
}

/// Build an A2AClient for the given agent + optional bearer token.
pub async fn build_client(
    resolved: &ResolvedAgent,
    bearer: Option<&str>,
    binding: Option<&str>,
) -> Result<A2AClient> {
    let card = fetch_card(&resolved.url, bearer).await?;

    let mut factory_builder = A2AClientFactory::builder();
    if let Some(b) = binding {
        factory_builder = factory_builder.preferred_bindings(vec![b.to_string()]);
    }
    if let Some(token) = bearer {
        factory_builder =
            factory_builder.with_interceptor(Arc::new(AuthInterceptor::bearer(token)));
    }
    let factory = factory_builder.build();
    factory.create_from_card(&card).await.map_err(AgcError::A2A)
}

fn build_http_client(bearer: Option<&str>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder();
    if let Some(token) = bearer {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {token}")
                .parse()
                .map_err(|e| AgcError::Auth(format!("invalid bearer token: {e}")))?,
        );
        builder = builder.default_headers(headers);
    }
    builder.build().map_err(AgcError::Http)
}
