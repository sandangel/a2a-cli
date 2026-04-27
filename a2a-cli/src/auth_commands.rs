//! Stub for gws-cli's auth_commands module.
//! credential_store.rs references crate::auth_commands::config_dir() —
//! we redirect it to our own config directory (~/.config/a2a-cli).

use std::path::PathBuf;

pub fn config_dir() -> PathBuf {
    crate::config::config_dir().unwrap_or_else(|_| {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("a2a-cli")
    })
}
