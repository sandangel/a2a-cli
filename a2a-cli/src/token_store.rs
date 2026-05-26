//! Per-agent OAuth token storage.
//!
//! Storage priority:
//!   1. OS keyring (macOS Keychain, Linux Secret Service, Windows Credential Manager)
//!      — service: "a2a-cli", key: canonical agent URL fingerprint
//!   2. AES-256-GCM encrypted file at ~/.config/a2a-cli/tokens/<agent-key>.enc
//!      — used when keyring unavailable or A2A_KEYRING_BACKEND=file
//!      — AES key stored in keyring under service "a2a-cli", key "encryption-key"

use std::path::PathBuf;
use std::sync::OnceLock;

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Nonce};
use rand::RngCore;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::config_dir;
use crate::error::{A2aCliError, Result};
use crate::fs_util::atomic_write;

const KEYRING_SERVICE: &str = "a2a-cli";
const KEYRING_ENC_KEY: &str = "encryption-key";
const STORAGE_KEY_PREFIX_MAX: usize = 80;

// ── Token type ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenGrantType {
    AuthorizationCode,
    DeviceCode,
    ClientCredentials,
}

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
    /// Token endpoint URL — stored so we can refresh without re-fetching the agent card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_url: Option<String>,
    /// OAuth client ID used for the token. Stored so raw-URL logins can refresh later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// OAuth grant type used to obtain the token. Stored so grant-specific renewal works.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_type: Option<TokenGrantType>,
}

impl Token {
    pub fn is_expired(&self) -> bool {
        let Some(exp) = self.expires_at else {
            return false;
        };
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
        let backend = std::env::var("A2A_KEYRING_BACKEND")
            .unwrap_or_default()
            .to_lowercase();
        match backend.as_str() {
            "keyring" => Backend::Keyring,
            "file" => Backend::File,
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
    let key = token_storage_key(agent_url)?;
    if let Ok(json) = load_token_json(&key) {
        return parse_token_json(&json).map(Some);
    }

    let legacy_key = legacy_token_key(agent_url)?;
    if legacy_key != key {
        return load_and_migrate_legacy_token(&legacy_key, &key);
    }

    Ok(None)
}

pub fn save_token(agent_url: &str, token: &Token) -> Result<()> {
    let key = token_storage_key(agent_url)?;
    let json = serde_json::to_string(token)
        .map_err(|e| A2aCliError::Auth(format!("serialize token: {e}")))?;
    save_token_json(&key, &json)
}

pub fn delete_token(agent_url: &str) -> Result<()> {
    let key = token_storage_key(agent_url)?;
    delete_token_key(&key)?;

    let legacy_key = legacy_token_key(agent_url)?;
    if legacy_key != key {
        delete_token_key(&legacy_key)?;
    }

    Ok(())
}

fn load_token_json(key: &str) -> anyhow::Result<String> {
    match Backend::from_env() {
        Backend::Keyring => keyring_load(key).or_else(|e| {
            eprintln!("warning: keyring unavailable ({e}), falling back to encrypted file");
            file_load(key)
        }),
        Backend::File => file_load(key),
    }
}

fn save_token_json(key: &str, json: &str) -> Result<()> {
    match Backend::from_env() {
        Backend::Keyring => keyring_save(key, json).or_else(|e| {
            eprintln!("warning: keyring save failed ({e}), using encrypted file instead");
            file_save(key, json)
        }),
        Backend::File => file_save(key, json),
    }
}

fn delete_token_key(key: &str) -> Result<()> {
    let _ = keyring_delete(key);
    let path = token_path(key)?;
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| A2aCliError::Auth(format!("delete token file: {e}")))?;
    }
    Ok(())
}

fn parse_token_json(json: &str) -> Result<Token> {
    serde_json::from_str(json).map_err(|e| A2aCliError::Auth(format!("parse token: {e}")))
}

fn load_and_migrate_legacy_token(legacy_key: &str, key: &str) -> Result<Option<Token>> {
    let Ok(json) = load_token_json(legacy_key) else {
        return Ok(None);
    };
    let token = parse_token_json(&json)?;

    save_token_json(key, &json)?;
    delete_token_key(legacy_key)?;

    Ok(Some(token))
}

// ── Keyring backend ───────────────────────────────────────────────────

fn keyring_load(key: &str) -> anyhow::Result<String> {
    Ok(keyring::Entry::new(KEYRING_SERVICE, key)?.get_password()?)
}

fn keyring_save(key: &str, json: &str) -> Result<()> {
    keyring::Entry::new(KEYRING_SERVICE, key)
        .map_err(|e| A2aCliError::Auth(format!("keyring: {e}")))?
        .set_password(json)
        .map_err(|e| A2aCliError::Auth(format!("keyring save: {e}")))
}

fn keyring_delete(key: &str) -> Result<()> {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, key) {
        let _ = entry.delete_credential();
    }
    Ok(())
}

// ── File backend ──────────────────────────────────────────────────────

fn token_path(key: &str) -> Result<PathBuf> {
    Ok(config_dir()?.join("tokens").join(format!("{key}.enc")))
}

fn file_load(key: &str) -> anyhow::Result<String> {
    let ciphertext = std::fs::read(token_path(key)?)?;
    let plaintext = aes_decrypt(&ciphertext)?;
    Ok(String::from_utf8(plaintext)?)
}

fn file_save(key: &str, json: &str) -> Result<()> {
    let path = token_path(key)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(A2aCliError::Io)?;
    }
    let ciphertext =
        aes_encrypt(json.as_bytes()).map_err(|e| A2aCliError::Auth(format!("encrypt: {e}")))?;
    atomic_write(&path, &ciphertext).map_err(|e| A2aCliError::Auth(format!("write token: {e}")))
}

// ── AES-256-GCM (key stored under service "a2a-cli") ──────────────────

fn aes_encrypt(plaintext: &[u8]) -> anyhow::Result<Vec<u8>> {
    let key = get_or_create_aes_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key)?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let mut out = nonce.to_vec();
    out.extend(
        cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| anyhow::anyhow!("encrypt failed"))?,
    );
    Ok(out)
}

fn aes_decrypt(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    if data.len() < 12 {
        anyhow::bail!("encrypted data too short");
    }
    let key = get_or_create_aes_key()?;
    aes_decrypt_with_key(data, &key)
}

fn aes_decrypt_with_key(data: &[u8], key: &[u8; 32]) -> anyhow::Result<Vec<u8>> {
    if data.len() < 12 {
        anyhow::bail!("encrypted data too short");
    }
    let cipher = Aes256Gcm::new_from_slice(&key[..])?;
    let nonce = Nonce::from_slice(&data[..12]);
    cipher
        .decrypt(nonce, &data[12..])
        .map_err(|_| anyhow::anyhow!("decrypt failed"))
}

/// Load or generate the AES-256 key, stored in keyring under service "a2a-cli".
/// Falls back to ~/.config/a2a-cli/.encryption_key file if keyring unavailable.
fn get_or_create_aes_key() -> anyhow::Result<[u8; 32]> {
    static KEY: OnceLock<[u8; 32]> = OnceLock::new();
    if let Some(k) = KEY.get() {
        return Ok(*k);
    }

    let key_file = encryption_key_path()?;

    // 1. Try keyring.
    if Backend::from_env() == Backend::Keyring
        && let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ENC_KEY)
        && let Ok(b64) = entry.get_password()
        && let Some(arr) = decode_key(&b64)
    {
        let _ = KEY.set(arr);
        return Ok(arr);
    }

    // 2. Try key file.
    if key_file.exists() {
        let b64 = std::fs::read_to_string(&key_file)?;
        let arr = decode_key(&b64).ok_or_else(|| anyhow::anyhow!("invalid key length"))?;
        let _ = KEY.set(arr);
        return Ok(arr);
    }

    // 3. Generate new key, persist to keyring + file fallback.
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    persist_aes_key(&key)?;

    let _ = KEY.set(key);
    Ok(key)
}

fn encryption_key_path() -> Result<PathBuf> {
    Ok(config_dir()?.join(".encryption_key"))
}

fn persist_aes_key(key: &[u8; 32]) -> anyhow::Result<()> {
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key);

    if Backend::from_env() == Backend::Keyring
        && let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ENC_KEY)
    {
        let _ = entry.set_password(&b64);
    }

    // Always write file fallback (ensures key survives keyring loss).
    let key_file = encryption_key_path()?;
    if let Some(parent) = key_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = atomic_write(&key_file, b64.as_bytes());
    Ok(())
}

fn decode_key(b64: &str) -> Option<[u8; 32]> {
    let bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64.trim()).ok()?;
    bytes.try_into().ok()
}

// ── Helpers ───────────────────────────────────────────────────────────

fn token_storage_key(agent_url: &str) -> Result<String> {
    let url = canonical_agent_url(agent_url)?;
    let prefix = safe_host_prefix(&url)?;
    let digest = Sha256::digest(url.as_str().as_bytes());
    let fingerprint =
        base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, digest);

    Ok(format!("{prefix}-{fingerprint}"))
}

fn canonical_agent_url(agent_url: &str) -> Result<Url> {
    let mut url = Url::parse(agent_url)
        .map_err(|e| A2aCliError::Config(format!("invalid agent URL {agent_url:?}: {e}")))?;
    if url.host_str().is_none() {
        return Err(A2aCliError::Config(format!(
            "cannot extract host from URL: {agent_url}"
        )));
    }

    url.set_fragment(None);

    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(&path);

    Ok(url)
}

fn safe_host_prefix(url: &Url) -> Result<String> {
    let host = url.host_str().ok_or_else(|| {
        A2aCliError::Config(format!("cannot extract host from URL: {}", url.as_str()))
    })?;
    let host_port = match url.port() {
        Some(port) => format!("{host}_{port}"),
        None => host.to_string(),
    };
    let safe = host_port
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(STORAGE_KEY_PREFIX_MAX)
        .collect::<String>();

    if safe.is_empty() {
        return Err(A2aCliError::Config(format!(
            "cannot extract host from URL: {}",
            url.as_str()
        )));
    }

    Ok(safe)
}

fn legacy_token_key(url: &str) -> Result<String> {
    let stripped = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host = stripped.split('/').next().unwrap_or(stripped);
    if host.is_empty() {
        return Err(A2aCliError::Config(format!(
            "cannot extract host from URL: {url}"
        )));
    }
    Ok(host.replace(':', "_"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{OsStr, OsString};

    struct EnvGuard {
        name: &'static str,
        old: Option<OsString>,
    }

    impl EnvGuard {
        fn set(name: &'static str, value: impl AsRef<OsStr>) -> Self {
            let old = std::env::var_os(name);
            // SAFETY: tests using EnvGuard are annotated #[serial_test::serial],
            // so environment mutation is serialized.
            unsafe { std::env::set_var(name, value) };
            Self { name, old }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.old {
                // SAFETY: see EnvGuard::set.
                Some(value) => unsafe { std::env::set_var(self.name, value) },
                // SAFETY: see EnvGuard::set.
                None => unsafe { std::env::remove_var(self.name) },
            }
        }
    }

    fn token(access_token: &str) -> Token {
        Token {
            access_token: access_token.to_string(),
            refresh_token: None,
            expires_at: None,
            token_type: "Bearer".to_string(),
            scopes: Vec::new(),
            token_url: None,
            client_id: None,
            grant_type: None,
        }
    }

    fn save_legacy_token(agent_url: &str, token: &Token) -> String {
        let key = legacy_token_key(agent_url).unwrap();
        let json = serde_json::to_string(token).unwrap();
        save_token_json(&key, &json).unwrap();
        key
    }

    #[test]
    fn storage_key_separates_same_host_different_paths() {
        let first = token_storage_key("https://example.com/agent-a").unwrap();
        let second = token_storage_key("https://example.com/agent-b").unwrap();

        assert_ne!(first, second);
        assert!(first.starts_with("example.com-"));
        assert!(second.starts_with("example.com-"));
    }

    #[test]
    fn storage_key_normalizes_trailing_slash() {
        let without_slash = token_storage_key("https://example.com/agent").unwrap();
        let with_slash = token_storage_key("https://example.com/agent/").unwrap();

        assert_eq!(without_slash, with_slash);
    }

    #[test]
    #[serial_test::serial]
    fn file_backend_separates_tokens_for_same_host_different_paths() {
        let dir = tempfile::tempdir().unwrap();
        let _config_dir = EnvGuard::set("A2A_CONFIG_DIR", dir.path());
        let _backend = EnvGuard::set("A2A_KEYRING_BACKEND", "file");

        save_token("https://example.com/agent-a", &token("first")).unwrap();
        save_token("https://example.com/agent-b", &token("second")).unwrap();

        let first = load_token("https://example.com/agent-a").unwrap().unwrap();
        let second = load_token("https://example.com/agent-b").unwrap().unwrap();

        assert_eq!(first.access_token, "first");
        assert_eq!(second.access_token, "second");
    }

    #[test]
    #[serial_test::serial]
    fn root_legacy_token_migrates_to_new_storage_key() {
        let dir = tempfile::tempdir().unwrap();
        let _config_dir = EnvGuard::set("A2A_CONFIG_DIR", dir.path());
        let _backend = EnvGuard::set("A2A_KEYRING_BACKEND", "file");
        let legacy_key = save_legacy_token("https://example.com", &token("legacy"));

        let loaded = load_token("https://example.com").unwrap().unwrap();

        assert_eq!(loaded.access_token, "legacy");
        assert!(file_load(&token_storage_key("https://example.com").unwrap()).is_ok());
        assert!(file_load(&legacy_key).is_err());
    }

    #[test]
    #[serial_test::serial]
    fn path_legacy_token_migrates_to_new_storage_key() {
        let dir = tempfile::tempdir().unwrap();
        let _config_dir = EnvGuard::set("A2A_CONFIG_DIR", dir.path());
        let _backend = EnvGuard::set("A2A_KEYRING_BACKEND", "file");
        let legacy_key = save_legacy_token("https://example.com/agent-a", &token("legacy"));

        let loaded = load_token("https://example.com/agent-a").unwrap().unwrap();

        assert_eq!(loaded.access_token, "legacy");
        assert!(file_load(&token_storage_key("https://example.com/agent-a").unwrap()).is_ok());
        assert!(file_load(&legacy_key).is_err());
    }

    #[test]
    #[serial_test::serial]
    fn same_host_legacy_token_is_consumed_by_first_migration() {
        let dir = tempfile::tempdir().unwrap();
        let _config_dir = EnvGuard::set("A2A_CONFIG_DIR", dir.path());
        let _backend = EnvGuard::set("A2A_KEYRING_BACKEND", "file");
        let legacy_key = save_legacy_token("https://example.com/agent-a", &token("legacy"));

        let first = load_token("https://example.com/agent-a").unwrap().unwrap();
        let second = load_token("https://example.com/agent-b").unwrap();

        assert_eq!(first.access_token, "legacy");
        assert!(second.is_none());
        assert!(file_load(&token_storage_key("https://example.com/agent-a").unwrap()).is_ok());
        assert!(file_load(&token_storage_key("https://example.com/agent-b").unwrap()).is_err());
        assert!(file_load(&legacy_key).is_err());
    }

    #[test]
    #[serial_test::serial]
    fn delete_token_removes_legacy_source_to_make_logout_stick() {
        let dir = tempfile::tempdir().unwrap();
        let _config_dir = EnvGuard::set("A2A_CONFIG_DIR", dir.path());
        let _backend = EnvGuard::set("A2A_KEYRING_BACKEND", "file");
        let legacy_key = save_legacy_token("https://example.com/agent-a", &token("legacy"));

        let loaded = load_token("https://example.com/agent-a").unwrap().unwrap();
        assert_eq!(loaded.access_token, "legacy");

        delete_token("https://example.com/agent-a").unwrap();

        let loaded = load_token("https://example.com/agent-a").unwrap();
        assert!(loaded.is_none());
        assert!(file_load(&token_storage_key("https://example.com/agent-a").unwrap()).is_err());
        assert!(file_load(&legacy_key).is_err());
    }
}
