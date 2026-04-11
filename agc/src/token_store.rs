//! Per-agent OAuth token storage.
//!
//! Storage priority (mirrors Go agc):
//!   1. OS keyring (macOS Keychain, Linux Secret Service, Windows Credential Manager)
//!      — service: "agc", key: agent URL hostname
//!   2. AES-256-GCM encrypted file at ~/.config/agc/tokens/<host>.enc
//!      — used when keyring unavailable or AGC_KEYRING_BACKEND=file
//!      — AES key stored in keyring under service "agc", key "encryption-key"
//!
//! Token is stored as JSON. All keyring entries use service "agc" (not "gws-cli").

use std::path::PathBuf;
use std::sync::OnceLock;

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::auth_commands::config_dir;
use crate::error::{AgcError, Result};
use crate::fs_util::atomic_write;

const KEYRING_SERVICE: &str = "agc";
const KEYRING_ENC_KEY: &str = "encryption-key";

// ── Token type ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>, // unix timestamp seconds
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
}

impl Token {
    pub fn is_expired(&self) -> bool {
        let Some(exp) = self.expires_at else { return false };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now + 60 >= exp // 60-second buffer
    }
}

// ── Backend selection ─────────────────────────────────────────────────

#[derive(PartialEq)]
enum Backend {
    Keyring,
    File,
}

impl Backend {
    fn from_env() -> Self {
        match std::env::var("AGC_KEYRING_BACKEND").as_deref() {
            Ok("keyring") => Backend::Keyring,
            Ok("file")    => Backend::File,
            // On Linux the keyring crate has no real keychain without explicit
            // D-Bus features (mirrors gws-cli behaviour: file-only on Linux).
            // On macOS/Windows the native keychain is reliable.
            _ => {
                if cfg!(any(target_os = "macos", target_os = "windows")) {
                    Backend::Keyring
                } else {
                    Backend::File
                }
            }
        }
    }
}

// ── Public API ────────────────────────────────────────────────────────

pub fn load_token(agent_url: &str) -> Result<Option<Token>> {
    let host = url_host(agent_url)?;
    let json = match Backend::from_env() {
        Backend::Keyring => keyring_load(&host).or_else(|_| file_load(&host)),
        Backend::File    => file_load(&host),
    };
    match json {
        Ok(s) => Ok(Some(
            serde_json::from_str(&s).map_err(|e| AgcError::Auth(format!("parse token: {e}")))?,
        )),
        Err(_) => Ok(None),
    }
}

pub fn save_token(agent_url: &str, token: &Token) -> Result<()> {
    let host = url_host(agent_url)?;
    let json = serde_json::to_string(token)
        .map_err(|e| AgcError::Auth(format!("serialize token: {e}")))?;
    match Backend::from_env() {
        Backend::Keyring => keyring_save(&host, &json).or_else(|_| file_save(&host, &json)),
        Backend::File    => file_save(&host, &json),
    }
}

pub fn delete_token(agent_url: &str) -> Result<()> {
    let host = url_host(agent_url)?;
    let _ = keyring_delete(&host);
    let path = token_path(&host);
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| AgcError::Auth(format!("delete token file: {e}")))?;
    }
    Ok(())
}

// ── Keyring backend ───────────────────────────────────────────────────

fn keyring_load(host: &str) -> anyhow::Result<String> {
    Ok(keyring::Entry::new(KEYRING_SERVICE, host)?.get_password()?)
}

fn keyring_save(host: &str, json: &str) -> Result<()> {
    keyring::Entry::new(KEYRING_SERVICE, host)
        .map_err(|e| AgcError::Auth(format!("keyring: {e}")))?
        .set_password(json)
        .map_err(|e| AgcError::Auth(format!("keyring save: {e}")))
}

fn keyring_delete(host: &str) -> Result<()> {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, host) {
        let _ = entry.delete_credential();
    }
    Ok(())
}

// ── File backend ──────────────────────────────────────────────────────

fn token_path(host: &str) -> PathBuf {
    config_dir().join("tokens").join(format!("{host}.enc"))
}

fn file_load(host: &str) -> anyhow::Result<String> {
    let ciphertext = std::fs::read(token_path(host))?;
    let plaintext = aes_decrypt(&ciphertext)?;
    Ok(String::from_utf8(plaintext)?)
}

fn file_save(host: &str, json: &str) -> Result<()> {
    let path = token_path(host);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(AgcError::Io)?;
    }
    let ciphertext =
        aes_encrypt(json.as_bytes()).map_err(|e| AgcError::Auth(format!("encrypt: {e}")))?;
    atomic_write(&path, &ciphertext)
        .map_err(|e| AgcError::Auth(format!("write token: {e}")))
}

// ── AES-256-GCM (key stored under service "agc") ──────────────────────

fn aes_encrypt(plaintext: &[u8]) -> anyhow::Result<Vec<u8>> {
    let key = get_or_create_aes_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key)?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let mut out = nonce.to_vec();
    out.extend(cipher.encrypt(&nonce, plaintext).map_err(|_| anyhow::anyhow!("encrypt failed"))?);
    Ok(out)
}

fn aes_decrypt(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    if data.len() < 12 {
        anyhow::bail!("encrypted data too short");
    }
    let key = get_or_create_aes_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key)?;
    let nonce = Nonce::from_slice(&data[..12]);
    cipher.decrypt(nonce, &data[12..]).map_err(|_| anyhow::anyhow!("decrypt failed"))
}

/// Load or generate the AES-256 key, stored in keyring under service "agc".
/// Falls back to ~/.config/agc/.encryption_key file if keyring unavailable.
fn get_or_create_aes_key() -> anyhow::Result<[u8; 32]> {
    static KEY: OnceLock<[u8; 32]> = OnceLock::new();
    if let Some(k) = KEY.get() {
        return Ok(*k);
    }

    let key_file = config_dir().join(".encryption_key");

    // 1. Try keyring.
    if Backend::from_env() == Backend::Keyring {
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ENC_KEY) {
            if let Ok(b64) = entry.get_password() {
                if let Ok(bytes) = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD, b64.trim()
                ) {
                    if let Ok(arr) = bytes.try_into() {
                        let _ = KEY.set(arr);
                        return Ok(arr);
                    }
                }
            }
        }
    }

    // 2. Try key file.
    if key_file.exists() {
        let b64 = std::fs::read_to_string(&key_file)?;
        let bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD, b64.trim()
        )?;
        let arr: [u8; 32] = bytes.try_into().map_err(|_| anyhow::anyhow!("invalid key length"))?;
        let _ = KEY.set(arr);
        return Ok(arr);
    }

    // 3. Generate new key, persist to keyring + file fallback.
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key);

    if Backend::from_env() == Backend::Keyring {
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ENC_KEY) {
            let _ = entry.set_password(&b64);
        }
    }

    // Always write file fallback (ensures key survives keyring loss).
    if let Some(parent) = key_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = atomic_write(&key_file, b64.as_bytes());

    let _ = KEY.set(key);
    Ok(key)
}

// ── Helpers ───────────────────────────────────────────────────────────

fn url_host(url: &str) -> Result<String> {
    let stripped = url.trim_start_matches("https://").trim_start_matches("http://");
    let host = stripped.split('/').next().unwrap_or(stripped);
    if host.is_empty() {
        return Err(AgcError::Config(format!("cannot extract host from URL: {url}")));
    }
    Ok(host.replace(':', "_"))
}
