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

    #[serde(default, skip_serializing_if = "OAuthConfig::is_empty")]
    pub oauth: OAuthConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OAuthConfig {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client_id: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
}

impl OAuthConfig {
    fn is_empty(&self) -> bool {
        self.client_id.is_empty() && self.scopes.is_empty()
    }
}

// ── Path helpers ──────────────────────────────────────────────────────

pub fn config_dir() -> Result<PathBuf> {
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
    serde_yaml::from_str(&data)
        .map_err(|e| AgcError::Config(format!("parse config: {e}")))
}

pub fn save(cfg: &Config) -> Result<()> {
    let path = config_path()?;
    let data = serde_yaml::to_string(cfg)
        .map_err(|e| AgcError::Config(format!("serialize config: {e}")))?;
    atomic_write(&path, data.as_bytes())
        .map_err(|e| AgcError::Config(format!("write config: {e}")))
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
                oauth: OAuthConfig::default(),
            });
        }
        None
    }

    pub fn active_agent(&self) -> Option<(&str, &Agent)> {
        if self.current_agent.is_empty() {
            return None;
        }
        self.agents
            .get(&self.current_agent)
            .map(|a| (self.current_agent.as_str(), a))
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
            oauth: OAuthConfig {
                client_id: format!("https://{host}/a2a/agc/.well-known/client-metadata.json"),
                scopes: vec![],
            },
        },
    );
    Config {
        current_agent: "rover".to_string(),
        agents,
    }
}
