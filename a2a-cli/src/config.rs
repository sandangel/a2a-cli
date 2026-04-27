//! Config file — ~/.config/a2a-cli/config.yaml

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{A2aCliError, Result};
use crate::fs_util::atomic_write;
use crate::validate::AgentAlias;

const CONFIG_DIR: &str = "a2a-cli";
const CONFIG_FILE: &str = "config.yaml";
const CLIENT_METADATA_PATH: &str = "/a2a/a2a-cli/.well-known/client-metadata.json";

// ── Types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// The active agent alias. `None` means no agent has been set via `a2a agent use`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_agent: Option<AgentAlias>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub agents: HashMap<AgentAlias, Agent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub url: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,

    /// Preferred transport: "jsonrpc" or "http-json". Empty = auto from agent card.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub transport: String,

    /// OAuth config — `None` means no OAuth configured for this agent.
    /// Distinguishes "not configured" from "configured with empty values".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthConfig>,
}

impl Agent {
    /// Return a reference to the OAuth config, or the shared empty sentinel if not configured.
    /// Callers should prefer this over unwrapping `oauth` directly.
    pub fn oauth_or_default(&self) -> &OAuthConfig {
        static EMPTY: OAuthConfig = OAuthConfig::EMPTY;
        self.oauth.as_ref().unwrap_or(&EMPTY)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OAuthConfig {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client_id: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
}

impl OAuthConfig {
    /// Shared empty instance used as a fallback when OAuth is not configured.
    pub const EMPTY: Self = Self {
        client_id: String::new(),
        scopes: Vec::new(),
    };
}

// ── Path helpers ──────────────────────────────────────────────────────

pub fn config_dir() -> Result<PathBuf> {
    // Allow tests (and headless environments) to redirect config to an isolated
    // temp directory so they don't pollute the developer's real config.
    if let Some(dir) = config_override_dir() {
        return Ok(dir);
    }
    default_config_dir()
}

fn config_override_dir() -> Option<PathBuf> {
    std::env::var_os("A2A_CONFIG_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn default_config_dir() -> Result<PathBuf> {
    let base = dirs::config_dir()
        .ok_or_else(|| A2aCliError::Config("cannot locate config directory".to_string()))?;
    Ok(base.join(CONFIG_DIR))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join(CONFIG_FILE))
}

// ── Load / Save ───────────────────────────────────────────────────────

pub fn load() -> Result<Config> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(default_config());
    }
    load_config_at(&path)
}

fn load_config_at(path: &std::path::Path) -> Result<Config> {
    let data = std::fs::read_to_string(path)
        .map_err(|e| A2aCliError::Config(format!("read config {}: {e}", path.display())))?;
    let cfg: Config = serde_yaml::from_str(&data)
        .map_err(|e| A2aCliError::Config(format!("parse config: {e}")))?;
    cfg.check_invariants()?;
    Ok(cfg)
}

pub fn save(cfg: &Config) -> Result<()> {
    let path = config_path()?;
    let data = serde_yaml::to_string(cfg)
        .map_err(|e| A2aCliError::Config(format!("serialize config: {e}")))?;
    atomic_write(&path, data.as_bytes())
        .map_err(|e| A2aCliError::Config(format!("write config: {e}")))
}

// ── Helpers on Config ─────────────────────────────────────────────────

impl Config {
    /// Resolve `--agent` value: alias lookup first, then raw URL.
    /// Accepts `&str` directly via `AgentAlias: Borrow<str>`.
    pub fn resolve_agent(&self, name_or_url: &str) -> Option<Agent> {
        if let Some(a) = self.agents.get(name_or_url) {
            return Some(a.clone());
        }
        if name_or_url.starts_with("http://") || name_or_url.starts_with("https://") {
            return Some(Agent {
                url: name_or_url.to_string(),
                description: String::new(),
                transport: String::new(),
                oauth: None,
            });
        }
        None
    }

    /// Return the active agent `(alias_str, &Agent)`, or `None` if none is set.
    pub fn active_agent(&self) -> Option<(&str, &Agent)> {
        let alias = self.current_agent.as_ref()?;
        let agent = self.agents.get(alias.as_str())?;
        Some((alias.as_str(), agent))
    }

    /// Assert the aggregate invariant: if `current_agent` is set it must exist in `agents`.
    pub(crate) fn check_invariants(&self) -> crate::error::Result<()> {
        if let Some(alias) = &self.current_agent
            && !self.agents.contains_key(alias.as_str())
        {
            return Err(crate::error::A2aCliError::Config(format!(
                "active agent {alias:?} is not registered — run: a2a agent add {alias:?} <url>",
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_agent(url: &str) -> Agent {
        Agent {
            url: url.to_string(),
            description: String::new(),
            transport: String::new(),
            oauth: None,
        }
    }

    // ── Config::resolve_agent ─────────────────────────────────────────

    #[test]
    fn resolve_agent_by_known_alias() {
        let mut cfg = Config::default();
        cfg.agents.insert(
            AgentAlias::new("prod").unwrap(),
            make_agent("https://prod.example.com"),
        );
        let a = cfg.resolve_agent("prod").unwrap();
        assert_eq!(a.url, "https://prod.example.com");
    }

    #[test]
    fn resolve_agent_by_https_url_returns_inline_agent() {
        let cfg = Config::default();
        let a = cfg.resolve_agent("https://example.com/agent").unwrap();
        assert_eq!(a.url, "https://example.com/agent");
    }

    #[test]
    fn resolve_agent_by_http_url() {
        let cfg = Config::default();
        let a = cfg.resolve_agent("http://localhost:8080").unwrap();
        assert_eq!(a.url, "http://localhost:8080");
    }

    #[test]
    fn resolve_agent_unknown_alias_returns_none() {
        let cfg = Config::default();
        assert!(cfg.resolve_agent("unknown").is_none());
    }

    // ── Config::active_agent ──────────────────────────────────────────

    #[test]
    fn active_agent_returns_current() {
        let mut cfg = Config::default();
        let alias = AgentAlias::new("prod").unwrap();
        cfg.current_agent = Some(alias.clone());
        cfg.agents
            .insert(alias, make_agent("https://prod.example.com"));
        let (a, agent) = cfg.active_agent().unwrap();
        assert_eq!(a, "prod");
        assert_eq!(agent.url, "https://prod.example.com");
    }

    #[test]
    fn active_agent_none_current_returns_none() {
        let cfg = Config::default();
        assert!(cfg.active_agent().is_none());
    }

    #[test]
    fn active_agent_alias_missing_from_map_returns_none() {
        let mut cfg = Config::default();
        cfg.current_agent = Some(AgentAlias::new("ghost").unwrap());
        assert!(cfg.active_agent().is_none());
    }

    // ── OAuthConfig ───────────────────────────────────────────────────

    #[test]
    fn agent_without_oauth_roundtrips() {
        let agent = make_agent("https://example.com");
        let yaml = serde_yaml::to_string(&agent).unwrap();
        assert!(!yaml.contains("oauth"), "oauth absent when None: {yaml}");
        let back: Agent = serde_yaml::from_str(&yaml).unwrap();
        assert!(back.oauth.is_none());
    }

    #[test]
    fn oauth_or_default_returns_empty_when_none() {
        let agent = make_agent("https://example.com");
        assert!(agent.oauth_or_default().client_id.is_empty());
    }

    // ── Serde roundtrip ───────────────────────────────────────────────

    #[test]
    fn config_yaml_roundtrip() {
        let test_alias = AgentAlias::new("test").unwrap();
        let mut cfg = Config {
            current_agent: Some(test_alias.clone()),
            agents: HashMap::new(),
        };
        cfg.agents.insert(
            test_alias,
            Agent {
                url: "https://example.com".to_string(),
                description: "Test agent".to_string(),
                transport: "jsonrpc".to_string(),
                oauth: Some(OAuthConfig {
                    client_id: "client".to_string(),
                    scopes: vec!["openid".to_string(), "email".to_string()],
                }),
            },
        );

        let yaml = serde_yaml::to_string(&cfg).unwrap();
        let back: Config = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(back.current_agent.as_ref().unwrap().as_str(), "test");
        let agent = back.agents.get("test").unwrap();
        assert_eq!(agent.url, "https://example.com");
        assert_eq!(agent.transport, "jsonrpc");
        let oauth = agent.oauth.as_ref().unwrap();
        assert_eq!(oauth.client_id, "client");
        assert_eq!(oauth.scopes, ["openid", "email"]);
    }

    #[test]
    fn empty_config_roundtrip_omits_empty_fields() {
        let cfg = Config::default();
        let yaml = serde_yaml::to_string(&cfg).unwrap();
        // Empty config should serialize to minimal YAML (no noise).
        assert!(!yaml.contains("current_agent"));
        assert!(!yaml.contains("agents"));
    }
}

fn default_config() -> Config {
    let host = env!("A2A_DEFAULT_HOST");
    let mut agents = HashMap::new();
    // SAFETY: "rover" is a valid alias — static string, no path separators.
    let rover = AgentAlias::new("rover").expect("rover is a valid alias");
    agents.insert(
        rover.clone(),
        Agent {
            url: format!("https://{host}/a2a/rover-agent"),
            description: "Rover Agent".to_string(),
            transport: String::new(),
            oauth: Some(OAuthConfig {
                client_id: format!("https://{host}{CLIENT_METADATA_PATH}"),
                scopes: vec![],
            }),
        },
    );
    Config {
        current_agent: Some(rover),
        agents,
    }
}
