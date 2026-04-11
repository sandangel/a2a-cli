//! Stub for gws-cli's auth_commands module.
//! credential_store.rs references crate::auth_commands::config_dir() —
//! we redirect it to our own config directory (~/.config/agc).

use std::path::PathBuf;

pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("AGC_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("agc")
}
