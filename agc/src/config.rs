//! Config file — ~/.config/agc/config.yaml

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{AgcError, Result};
use crate::fs_util::atomic_write;

const CONFIG_DIR: &str = "agc";
const CONFIG_FILE: &str = "config.yaml";

// ── Types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub current_agent: String,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub agents: HashMap<String, Agent>,
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
    if let Ok(dir) = std::env::var("AGC_CONFIG_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let base = dirs::config_dir()
        .ok_or_else(|| AgcError::Config("cannot locate config directory".to_string()))?;
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
    let data = std::fs::read_to_string(&path)
        .map_err(|e| AgcError::Config(format!("read config: {e}")))?;
    let cfg: Config =
        serde_yaml::from_str(&data).map_err(|e| AgcError::Config(format!("parse config: {e}")))?;
    cfg.check_invariants()?;
    Ok(cfg)
}

pub fn save(cfg: &Config) -> Result<()> {
    let path = config_path()?;
    let data = serde_yaml::to_string(cfg)
        .map_err(|e| AgcError::Config(format!("serialize config: {e}")))?;
    atomic_write(&path, data.as_bytes()).map_err(|e| AgcError::Config(format!("write config: {e}")))
}

// ── Helpers on Config ─────────────────────────────────────────────────

impl Config {
    /// Resolve --agent value: alias lookup first, then raw URL.
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

    /// Return the active agent `(alias, &Agent)`, or `None` if none is set.
    ///
    /// Returns `None` in two cases:
    /// - `current_agent` is empty (no active agent configured)
    /// - `current_agent` names an alias not present in `agents` (broken aggregate
    ///   invariant — happens if an agent is removed without clearing the active alias)
    pub fn active_agent(&self) -> Option<(&str, &Agent)> {
        if self.current_agent.is_empty() {
            return None;
        }
        let agent = self.agents.get(&self.current_agent)?;
        // Invariant: current_agent must name a registered alias.
        // If it doesn't (stale reference after `agent remove`), treat as "no active
        // agent" so the caller gets a clear Config error from resolve_current_agent().
        Some((self.current_agent.as_str(), agent))
    }

    /// Assert the aggregate invariant: if `current_agent` is set it must exist in
    /// `agents`.  Call this after any mutation that could break the invariant.
    pub(crate) fn check_invariants(&self) -> crate::error::Result<()> {
        if !self.current_agent.is_empty() && !self.agents.contains_key(&self.current_agent) {
            return Err(crate::error::AgcError::Config(format!(
                "active agent {:?} is not registered — run: agc agent add {:?} <url>",
                self.current_agent, self.current_agent,
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
        cfg.agents
            .insert("prod".to_string(), make_agent("https://prod.example.com"));
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
        cfg.current_agent = "prod".to_string();
        cfg.agents
            .insert("prod".to_string(), make_agent("https://prod.example.com"));
        let (alias, agent) = cfg.active_agent().unwrap();
        assert_eq!(alias, "prod");
        assert_eq!(agent.url, "https://prod.example.com");
    }

    #[test]
    fn active_agent_empty_current_returns_none() {
        let cfg = Config::default();
        assert!(cfg.active_agent().is_none());
    }

    #[test]
    fn active_agent_alias_missing_from_map_returns_none() {
        let mut cfg = Config::default();
        cfg.current_agent = "ghost".to_string();
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
        let mut cfg = Config {
            current_agent: "test".to_string(),
            agents: HashMap::new(),
        };
        cfg.agents.insert(
            "test".to_string(),
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

        assert_eq!(back.current_agent, "test");
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
    let host = env!("AGC_DEFAULT_HOST");
    let mut agents = HashMap::new();
    agents.insert(
        "rover".to_string(),
        Agent {
            url: format!("https://{host}/a2a/rover-agent"),
            description: "Rover Agent".to_string(),
            transport: String::new(),
            oauth: Some(OAuthConfig {
                client_id: format!("https://{host}/a2a/agc/.well-known/client-metadata.json"),
                scopes: vec![],
            }),
        },
    );
    Config {
        current_agent: "rover".to_string(),
        agents,
    }
}
