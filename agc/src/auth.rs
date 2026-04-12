//! OAuth2 flows for A2A agents.
//!
//! Parses security schemes from raw card JSON (handles both v0.3 and v1 formats).
//! Supports: Authorization Code + PKCE, Device Code, Client Credentials, Bearer.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use oauth2::basic::BasicClient;
use oauth2::{
    AuthUrl, ClientId, ClientSecret, Scope,
    TokenUrl,
    devicecode::StandardDeviceAuthorizationResponse,
    DeviceAuthorizationUrl,
};
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::config::Agent;
use crate::error::{AgcError, Result};
use crate::token_store::{Token, delete_token, load_token, save_token};

// ── Extracted OAuth flow info (protocol-version agnostic) ─────────────

#[derive(Debug, Clone)]
pub enum OAuthFlow {
    AuthorizationCode { auth_url: String, token_url: String, scopes: Vec<String> },
    DeviceCode { device_auth_url: String, token_url: String, scopes: Vec<String> },
    ClientCredentials { token_url: String, scopes: Vec<String> },
}

/// Extract OAuth flows from a raw agent card JSON value.
/// Handles both A2A v0.3 (type: "oauth2") and v1 (oauth2SecurityScheme key) formats.
pub fn extract_oauth_flows(card: &Value) -> Vec<OAuthFlow> {
    let mut flows = vec![];
    let schemes = match card.get("securitySchemes").and_then(|s| s.as_object()) {
        Some(s) => s,
        None => return flows,
    };

    for (_name, scheme) in schemes {
        // v1 format: { "oauth2SecurityScheme": { "flows": { ... } } }
        let flows_val = scheme.get("oauth2SecurityScheme")
            .and_then(|o| o.get("flows"))
            // v0.3 format: { "type": "oauth2", "flows": { ... } }
            .or_else(|| {
                if scheme.get("type").and_then(|t| t.as_str()) == Some("oauth2") {
                    scheme.get("flows")
                } else {
                    None
                }
            });

        let Some(flows_obj) = flows_val.and_then(|f| f.as_object()) else { continue };

        // Authorization Code
        if let Some(ac) = flows_obj.get("authorizationCode")
            && let (Some(auth_url), Some(token_url)) = (
                ac.get("authorizationUrl").and_then(|v| v.as_str()),
                ac.get("tokenUrl").and_then(|v| v.as_str()),
            )
        {
            let scopes = extract_scope_names(ac.get("scopes"));
            flows.push(OAuthFlow::AuthorizationCode {
                auth_url: auth_url.to_string(),
                token_url: token_url.to_string(),
                scopes,
            });
        }

        // Device Code
        if let Some(dc) = flows_obj.get("deviceCode")
            && let (Some(device_url), Some(token_url)) = (
                dc.get("deviceAuthorizationUrl").and_then(|v| v.as_str()),
                dc.get("tokenUrl").and_then(|v| v.as_str()),
            )
        {
            let scopes = extract_scope_names(dc.get("scopes"));
            flows.push(OAuthFlow::DeviceCode {
                device_auth_url: device_url.to_string(),
                token_url: token_url.to_string(),
                scopes,
            });
        }

        // Client Credentials
        if let Some(cc) = flows_obj.get("clientCredentials")
            && let Some(token_url) = cc.get("tokenUrl").and_then(|v| v.as_str())
        {
            let scopes = extract_scope_names(cc.get("scopes"));
            flows.push(OAuthFlow::ClientCredentials {
                token_url: token_url.to_string(),
                scopes,
            });
        }
    }
    flows
}

fn extract_scope_names(scopes_val: Option<&Value>) -> Vec<String> {
    match scopes_val {
        Some(Value::Object(map)) => map.keys().cloned().collect(),
        Some(Value::Array(arr)) => arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        _ => vec![],
    }
}

// ── Public API ────────────────────────────────────────────────────────

pub async fn login(agent_url: &str, agent: &Agent, card: &Value) -> Result<Option<String>> {
    if let Some(token) = load_token(agent_url)?
        && !token.is_expired()
    {
        return Ok(Some(token.access_token));
    }

    let flows = extract_oauth_flows(card);
    if flows.is_empty() {
        return Ok(None);
    }

    // Prefer auth code, then device code, then client credentials — use first declared flow.
    let token = match flows.first() {
        Some(OAuthFlow::AuthorizationCode { auth_url, token_url, scopes }) => {
            let cfg_scopes = if agent.oauth.scopes.is_empty() { scopes } else { &agent.oauth.scopes };
            auth_code_pkce_flow(auth_url, token_url, &agent.oauth.client_id, cfg_scopes, agent_url).await?
        }
        Some(OAuthFlow::DeviceCode { device_auth_url, token_url, scopes }) => {
            let cfg_scopes = if agent.oauth.scopes.is_empty() { scopes } else { &agent.oauth.scopes };
            device_code_flow(device_auth_url, token_url, &agent.oauth.client_id, cfg_scopes, agent_url).await?
        }
        Some(OAuthFlow::ClientCredentials { token_url, scopes }) => {
            let cfg_scopes = if agent.oauth.scopes.is_empty() { scopes } else { &agent.oauth.scopes };
            client_credentials_flow(token_url, &agent.oauth.client_id, cfg_scopes, agent_url).await?
        }
        None => return Ok(None),
    };
    Ok(Some(token.access_token))
}

pub fn logout(agent_url: &str) -> Result<()> {
    delete_token(agent_url)
}

/// Return a valid access token, refreshing silently if expired.
///
/// - Not expired → return stored access token as-is.
/// - Expired + refresh_token + token_url stored → exchange for new token, save, return it.
/// - Expired but no refresh capability → return `None` (caller will get 401).
pub async fn refresh_if_expired(agent_url: &str, client_id: &str) -> Result<Option<String>> {
    let Some(token) = load_token(agent_url)? else {
        return Ok(None);
    };
    if !token.is_expired() {
        return Ok(Some(token.access_token));
    }
    let (Some(refresh_token), Some(token_url)) = (token.refresh_token.as_deref(), token.token_url.as_deref()) else {
        return Ok(None);
    };
    match do_refresh(token_url, client_id, refresh_token, &token.scopes, agent_url).await {
        Ok(new_token) => Ok(Some(new_token.access_token)),
        Err(e) => {
            eprintln!("warning: token refresh failed ({e}); re-run `agc auth login` to re-authenticate");
            Ok(None)
        }
    }
}

async fn do_refresh(
    token_url: &str,
    client_id: &str,
    refresh_token: &str,
    scopes: &[String],
    agent_url: &str,
) -> Result<Token> {
    let http = reqwest::Client::new();
    let resp = http.post(token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
        ])
        .send()
        .await
        .map_err(AgcError::Http)?;
    if !resp.status().is_success() {
        return Err(AgcError::Auth(format!("token refresh returned HTTP {}", resp.status())));
    }
    let body: Value = resp.json().await.map_err(AgcError::Http)?;
    // Preserve the existing refresh_token if the server doesn't issue a new one.
    let mut token = token_from_json(&body, scopes, Some(token_url))?;
    if token.refresh_token.is_none() {
        token.refresh_token = Some(refresh_token.to_string());
    }
    save_token(agent_url, &token)?;
    Ok(token)
}

pub struct TokenStatus {
    pub authenticated: bool,
    pub expired: bool,
    pub expires_at: Option<u64>,
    pub scopes: Vec<String>,
    pub masked_token: Option<String>,
}

pub fn token_status(agent_url: &str) -> Result<TokenStatus> {
    match load_token(agent_url)? {
        None => Ok(TokenStatus { authenticated: false, expired: false, expires_at: None, scopes: vec![], masked_token: None }),
        Some(t) => {
            let masked = Some(mask_token(&t.access_token));
            Ok(TokenStatus {
                authenticated: !t.is_expired(),
                expired: t.is_expired(),
                expires_at: t.expires_at,
                scopes: t.scopes.clone(),
                masked_token: masked,
            })
        }
    }
}

// ── Authorization Code + PKCE ─────────────────────────────────────────

async fn auth_code_pkce_flow(
    auth_url: &str,
    token_url: &str,
    client_id: &str,
    scopes: &[String],
    agent_url: &str,
) -> Result<Token> {
    // Bind a random local port for the redirect.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| AgcError::Auth(format!("bind local server: {e}")))?;
    let port = listener.local_addr().map_err(AgcError::Io)?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    // Generate PKCE verifier + challenge.
    let mut verifier_bytes = [0u8; 64];
    rand::thread_rng().fill_bytes(&mut verifier_bytes);
    let code_verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(verifier_bytes);
    let challenge_bytes = Sha256::digest(code_verifier.as_bytes());
    let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(challenge_bytes);

    // Generate state.
    let mut state_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut state_bytes);
    let state = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(state_bytes);

    // Build authorization URL.
    let mut params = vec![
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", &redirect_uri),
        ("code_challenge_method", "S256"),
    ];
    let scope_str = scopes.join(" ");
    if !scope_str.is_empty() { params.push(("scope", &scope_str)); }
    params.push(("code_challenge", &code_challenge));
    params.push(("state", &state));

    let full_auth_url = format!("{}?{}", auth_url,
        params.iter().map(|(k, v)| format!("{}={}", k, urlenccode(v))).collect::<Vec<_>>().join("&")
    );

    if open::that(&full_auth_url).is_ok() {
        eprintln!("\nOpening browser for authentication...");
        eprintln!("If the browser did not open, visit:\n\n  {full_auth_url}\n");
    } else {
        eprintln!("\nTo authenticate, open this URL in your browser:\n\n  {full_auth_url}\n");
    }

    // Wait for the callback.
    let (code, returned_state) = wait_for_callback(listener).await?;

    if returned_state != state {
        return Err(AgcError::Auth("OAuth state mismatch — possible CSRF".to_string()));
    }

    // Exchange code for token.
    let http = reqwest::Client::new();
    let resp = http.post(token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect_uri),
            ("client_id", client_id),
            ("code_verifier", &code_verifier),
        ])
        .send()
        .await
        .map_err(AgcError::Http)?;

    let body: Value = resp.json().await.map_err(AgcError::Http)?;
    let token = token_from_json(&body, scopes, Some(token_url))?;
    save_token(agent_url, &token)?;
    Ok(token)
}

async fn wait_for_callback(listener: tokio::net::TcpListener) -> Result<(String, String)> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    eprintln!("Waiting for browser redirect...\n");

    let (mut stream, _) = listener.accept().await
        .map_err(|e| AgcError::Auth(format!("accept callback: {e}")))?;

    let mut reader = BufReader::new(&mut stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).await
        .map_err(|e| AgcError::Auth(format!("read callback: {e}")))?;

    // Parse GET /callback?code=X&state=Y HTTP/1.1
    let path = request_line.split_whitespace().nth(1).unwrap_or("");
    let query = path.split('?').nth(1).unwrap_or("");
    let params: HashMap<_, _> = query.split('&')
        .filter_map(|p| { let mut s = p.splitn(2, '='); Some((s.next()?, s.next()?)) })
        .collect();

    let code = params.get("code")
        .ok_or_else(|| AgcError::Auth("no code in callback".to_string()))?
        .to_string();
    let state = params.get("state").copied().unwrap_or("").to_string();

    // Send success response to browser.
    let body = "<html><head><meta charset=\"utf-8\"></head><body><h2>Authentication successful - you can close this tab.</h2></body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(), body
    );
    let _ = stream.write_all(response.as_bytes()).await;

    Ok((code, state))
}

// ── Device Code Flow ──────────────────────────────────────────────────

async fn device_code_flow(
    device_auth_url: &str,
    token_url: &str,
    client_id: &str,
    scopes: &[String],
    agent_url: &str,
) -> Result<Token> {
    let client = BasicClient::new(
        ClientId::new(client_id.to_string()),
        None,
        AuthUrl::new("https://placeholder.invalid/auth".to_string())
            .map_err(|e| AgcError::Auth(format!("auth URL: {e}")))?,
        Some(TokenUrl::new(token_url.to_string())
            .map_err(|e| AgcError::Auth(format!("token URL: {e}")))?),
    )
    .set_device_authorization_url(
        DeviceAuthorizationUrl::new(device_auth_url.to_string())
            .map_err(|e| AgcError::Auth(format!("device auth URL: {e}")))?,
    );

    let details: StandardDeviceAuthorizationResponse = client
        .exchange_device_code()
        .map_err(|e| AgcError::Auth(format!("device code setup: {e}")))?
        .add_scopes(scopes.iter().map(|s| Scope::new(s.clone())))
        .request_async(oauth2::reqwest::async_http_client)
        .await
        .map_err(|e| AgcError::Auth(format!("device authorization: {e}")))?;

    eprintln!("\nTo authenticate, visit: {}", details.verification_uri().as_str());
    eprintln!("Enter code: {}\n", details.user_code().secret());

    let token_response = client
        .exchange_device_access_token(&details)
        .request_async(oauth2::reqwest::async_http_client, tokio::time::sleep, None)
        .await
        .map_err(|e| AgcError::Auth(format!("device token exchange: {e}")))?;

    use oauth2::TokenResponse;
    let token = Token {
        access_token: token_response.access_token().secret().clone(),
        refresh_token: token_response.refresh_token().map(|t| t.secret().clone()),
        expires_at: token_response.expires_in().map(|d| unix_now() + d.as_secs()),
        token_type: "Bearer".to_string(),
        scopes: scopes.to_vec(),
        token_url: Some(token_url.to_string()),
    };
    save_token(agent_url, &token)?;
    Ok(token)
}

// ── Client Credentials ────────────────────────────────────────────────

async fn client_credentials_flow(
    token_url: &str,
    client_id: &str,
    scopes: &[String],
    agent_url: &str,
) -> Result<Token> {
    let secret = std::env::var("AGC_CLIENT_SECRET")
        .map_err(|_| AgcError::Auth("client credentials requires AGC_CLIENT_SECRET".to_string()))?;

    let client = BasicClient::new(
        ClientId::new(client_id.to_string()),
        Some(ClientSecret::new(secret)),
        AuthUrl::new("https://placeholder.invalid/auth".to_string())
            .map_err(|e| AgcError::Auth(format!("auth URL: {e}")))?,
        Some(TokenUrl::new(token_url.to_string())
            .map_err(|e| AgcError::Auth(format!("token URL: {e}")))?),
    );

    use oauth2::TokenResponse;
    let resp = client
        .exchange_client_credentials()
        .add_scopes(scopes.iter().map(|s| Scope::new(s.clone())))
        .request_async(oauth2::reqwest::async_http_client)
        .await
        .map_err(|e| AgcError::Auth(format!("client credentials: {e}")))?;

    let token = Token {
        access_token: resp.access_token().secret().clone(),
        refresh_token: resp.refresh_token().map(|t| t.secret().clone()),
        expires_at: resp.expires_in().map(|d| unix_now() + d.as_secs()),
        token_type: "Bearer".to_string(),
        scopes: scopes.to_vec(),
        token_url: Some(token_url.to_string()),
    };
    save_token(agent_url, &token)?;
    Ok(token)
}

// ── Helpers ───────────────────────────────────────────────────────────

fn token_from_json(body: &Value, scopes: &[String], token_url: Option<&str>) -> Result<Token> {
    let access_token = body.get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AgcError::Auth(format!("no access_token in response: {body}")))?
        .to_string();

    let expires_at = body.get("expires_in")
        .and_then(|v| v.as_u64())
        .map(|secs| unix_now() + secs);

    Ok(Token {
        access_token,
        refresh_token: body.get("refresh_token").and_then(|v| v.as_str()).map(str::to_string),
        expires_at,
        token_type: body.get("token_type").and_then(|v| v.as_str()).unwrap_or("Bearer").to_string(),
        scopes: scopes.to_vec(),
        token_url: token_url.map(str::to_string),
    })
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card_with_auth_code() -> Value {
        serde_json::json!({
            "securitySchemes": {
                "myOAuth": {
                    "oauth2SecurityScheme": {
                        "flows": {
                            "authorizationCode": {
                                "authorizationUrl": "https://auth.example.com/authorize",
                                "tokenUrl": "https://auth.example.com/token",
                                "scopes": { "openid": "OpenID Connect", "email": "Email" }
                            }
                        }
                    }
                }
            }
        })
    }

    fn card_with_device_code() -> Value {
        serde_json::json!({
            "securitySchemes": {
                "myOAuth": {
                    "oauth2SecurityScheme": {
                        "flows": {
                            "deviceCode": {
                                "deviceAuthorizationUrl": "https://auth.example.com/device",
                                "tokenUrl": "https://auth.example.com/token",
                                "scopes": { "openid": "OpenID Connect" }
                            }
                        }
                    }
                }
            }
        })
    }

    fn card_with_client_credentials() -> Value {
        serde_json::json!({
            "securitySchemes": {
                "myOAuth": {
                    "oauth2SecurityScheme": {
                        "flows": {
                            "clientCredentials": {
                                "tokenUrl": "https://auth.example.com/token",
                                "scopes": { "api": "API access" }
                            }
                        }
                    }
                }
            }
        })
    }

    // ── extract_oauth_flows — v1 format ───────────────────────────────

    #[test]
    fn extract_v1_authorization_code_flow() {
        let flows = extract_oauth_flows(&card_with_auth_code());
        assert_eq!(flows.len(), 1);
        match &flows[0] {
            OAuthFlow::AuthorizationCode { auth_url, token_url, scopes } => {
                assert_eq!(auth_url, "https://auth.example.com/authorize");
                assert_eq!(token_url, "https://auth.example.com/token");
                assert!(scopes.contains(&"openid".to_string()));
                assert!(scopes.contains(&"email".to_string()));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn extract_v1_device_code_flow() {
        let flows = extract_oauth_flows(&card_with_device_code());
        assert_eq!(flows.len(), 1);
        match &flows[0] {
            OAuthFlow::DeviceCode { device_auth_url, token_url, scopes } => {
                assert_eq!(device_auth_url, "https://auth.example.com/device");
                assert_eq!(token_url, "https://auth.example.com/token");
                assert!(scopes.contains(&"openid".to_string()));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn extract_v1_client_credentials_flow() {
        let flows = extract_oauth_flows(&card_with_client_credentials());
        assert_eq!(flows.len(), 1);
        match &flows[0] {
            OAuthFlow::ClientCredentials { token_url, scopes } => {
                assert_eq!(token_url, "https://auth.example.com/token");
                assert!(scopes.contains(&"api".to_string()));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    // ── extract_oauth_flows — v0.3 format ─────────────────────────────

    #[test]
    fn extract_v03_type_oauth2_flow() {
        let card = serde_json::json!({
            "securitySchemes": {
                "myOAuth": {
                    "type": "oauth2",
                    "flows": {
                        "authorizationCode": {
                            "authorizationUrl": "https://auth.example.com/authorize",
                            "tokenUrl": "https://auth.example.com/token",
                            "scopes": {}
                        }
                    }
                }
            }
        });
        let flows = extract_oauth_flows(&card);
        assert_eq!(flows.len(), 1);
        assert!(matches!(flows[0], OAuthFlow::AuthorizationCode { .. }));
    }

    // ── edge cases ────────────────────────────────────────────────────

    #[test]
    fn extract_no_security_schemes_returns_empty() {
        let flows = extract_oauth_flows(&serde_json::json!({"name": "Agent"}));
        assert!(flows.is_empty());
    }

    #[test]
    fn extract_multiple_flows_from_same_scheme() {
        let card = serde_json::json!({
            "securitySchemes": {
                "myOAuth": {
                    "oauth2SecurityScheme": {
                        "flows": {
                            "authorizationCode": {
                                "authorizationUrl": "https://a.example.com/auth",
                                "tokenUrl": "https://a.example.com/token",
                                "scopes": {}
                            },
                            "deviceCode": {
                                "deviceAuthorizationUrl": "https://a.example.com/device",
                                "tokenUrl": "https://a.example.com/token",
                                "scopes": {}
                            }
                        }
                    }
                }
            }
        });
        let flows = extract_oauth_flows(&card);
        assert_eq!(flows.len(), 2);
    }

    #[test]
    fn extract_scopes_from_array_format() {
        let card = serde_json::json!({
            "securitySchemes": {
                "s": {
                    "oauth2SecurityScheme": {
                        "flows": {
                            "clientCredentials": {
                                "tokenUrl": "https://t.example.com/token",
                                "scopes": ["read", "write"]
                            }
                        }
                    }
                }
            }
        });
        let flows = extract_oauth_flows(&card);
        assert_eq!(flows.len(), 1);
        if let OAuthFlow::ClientCredentials { scopes, .. } = &flows[0] {
            assert!(scopes.contains(&"read".to_string()));
            assert!(scopes.contains(&"write".to_string()));
        }
    }

    #[test]
    fn extract_scheme_missing_flows_key_is_skipped() {
        let card = serde_json::json!({
            "securitySchemes": {
                "broken": { "oauth2SecurityScheme": {} }
            }
        });
        assert!(extract_oauth_flows(&card).is_empty());
    }
}

#[cfg(test)]
mod refresh_tests {
    use super::*;
    use crate::token_store::{load_token, save_token, Token};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// RAII guard: saves an env var on creation and restores it on drop.
    struct EnvGuard {
        name: &'static str,
        original: Option<std::ffi::OsString>,
    }
    impl EnvGuard {
        fn set(name: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let original = std::env::var_os(name);
            unsafe { std::env::set_var(name, value); }
            Self { name, original }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(v) => unsafe { std::env::set_var(self.name, v) },
                None    => unsafe { std::env::remove_var(self.name) },
            }
        }
    }

    fn unix_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn expired_token(refresh_token: Option<&str>, token_url: Option<&str>) -> Token {
        Token {
            access_token: "old-access".to_string(),
            refresh_token: refresh_token.map(str::to_string),
            expires_at: Some(unix_secs() - 300),
            token_type: "Bearer".to_string(),
            scopes: vec!["openid".to_string()],
            token_url: token_url.map(str::to_string),
        }
    }

    fn valid_token() -> Token {
        Token {
            access_token: "valid-access".to_string(),
            refresh_token: Some("valid-refresh".to_string()),
            expires_at: Some(unix_secs() + 3600),
            token_type: "Bearer".to_string(),
            scopes: vec!["openid".to_string()],
            token_url: Some("https://auth.example.com/token".to_string()),
        }
    }

    /// Spawn a minimal HTTP server on a random port that handles one POST request
    /// and responds with `status` + `response_body`.
    async fn spawn_token_server(
        status: u16,
        response_body: &'static str,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{port}/token");

        let handle = tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else { return };
            let (read_half, mut write_half) = tokio::io::split(stream);
            let mut reader = BufReader::new(read_half);

            // Read and discard request headers; track Content-Length.
            let mut content_length = 0usize;
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                    break;
                }
                if line == "\r\n" {
                    break;
                }
                let lower = line.to_lowercase();
                if let Some(v) = lower.strip_prefix("content-length: ") {
                    content_length = v.trim().parse().unwrap_or(0);
                }
            }
            // Read request body so the client doesn't see a connection reset.
            if content_length > 0 {
                let mut body_buf = vec![0u8; content_length];
                let _ = reader.read_exact(&mut body_buf).await;
            }

            let status_line = if status == 200 { "200 OK" } else { "400 Bad Request" };
            let resp = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            let _ = write_half.write_all(resp.as_bytes()).await;
        });

        (url, handle)
    }

    // ── refresh_if_expired ────────────────────────────────────────────

    #[tokio::test]
    #[serial_test::serial]
    async fn no_stored_token_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("AGC_CONFIG_DIR", dir.path());

        let result = refresh_if_expired("http://no-token.test/", "client-id").await;
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn valid_token_returned_without_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("AGC_CONFIG_DIR", dir.path());

        save_token("http://valid.test/", &valid_token()).unwrap();
        let result = refresh_if_expired("http://valid.test/", "client-id").await.unwrap();
        assert_eq!(result, Some("valid-access".to_string()));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn expired_no_refresh_token_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("AGC_CONFIG_DIR", dir.path());

        save_token(
            "http://no-refresh.test/",
            &expired_token(None, Some("https://auth.example.com/token")),
        )
        .unwrap();

        let result = refresh_if_expired("http://no-refresh.test/", "client-id").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn expired_no_token_url_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("AGC_CONFIG_DIR", dir.path());

        save_token(
            "http://no-token-url.test/",
            &expired_token(Some("old-refresh"), None),
        )
        .unwrap();

        let result = refresh_if_expired("http://no-token-url.test/", "client-id").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn expired_token_refreshes_successfully() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("AGC_CONFIG_DIR", dir.path());

        let (server_url, server) = spawn_token_server(
            200,
            r#"{"access_token":"new-access","expires_in":3600,"token_type":"Bearer","refresh_token":"new-refresh"}"#,
        )
        .await;

        save_token(
            "http://refresh-ok.test/",
            &expired_token(Some("old-refresh"), Some(&server_url)),
        )
        .unwrap();

        let result = refresh_if_expired("http://refresh-ok.test/", "client-id").await.unwrap();
        assert_eq!(result, Some("new-access".to_string()));

        // Verify the new token was persisted.
        let stored = load_token("http://refresh-ok.test/").unwrap().unwrap();
        assert_eq!(stored.access_token, "new-access");
        assert_eq!(stored.refresh_token.as_deref(), Some("new-refresh"));

        let _ = server.await;
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn expired_token_server_error_returns_none_with_warning() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("AGC_CONFIG_DIR", dir.path());

        let (server_url, server) =
            spawn_token_server(400, r#"{"error":"invalid_grant"}"#).await;

        save_token(
            "http://refresh-fail.test/",
            &expired_token(Some("bad-refresh"), Some(&server_url)),
        )
        .unwrap();

        let result = refresh_if_expired("http://refresh-fail.test/", "client-id")
            .await
            .unwrap();
        assert!(result.is_none(), "server error must return None");

        let _ = server.await;
    }

    // ── do_refresh ────────────────────────────────────────────────────

    #[tokio::test]
    #[serial_test::serial]
    async fn do_refresh_preserves_refresh_token_when_server_omits_it() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("AGC_CONFIG_DIR", dir.path());

        // Server returns access token but no refresh_token.
        let (server_url, server) = spawn_token_server(
            200,
            r#"{"access_token":"new-access","expires_in":3600,"token_type":"Bearer"}"#,
        )
        .await;

        let token = do_refresh(
            &server_url,
            "client-id",
            "original-refresh",
            &["openid".to_string()],
            "http://preserve-refresh.test/",
        )
        .await
        .unwrap();

        assert_eq!(token.access_token, "new-access");
        assert_eq!(
            token.refresh_token.as_deref(),
            Some("original-refresh"),
            "original refresh_token must be preserved"
        );

        let _ = server.await;
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn do_refresh_uses_new_refresh_token_when_server_returns_it() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("AGC_CONFIG_DIR", dir.path());

        let (server_url, server) = spawn_token_server(
            200,
            r#"{"access_token":"new-access","expires_in":3600,"token_type":"Bearer","refresh_token":"rotated-refresh"}"#,
        )
        .await;

        let token = do_refresh(
            &server_url,
            "client-id",
            "original-refresh",
            &["openid".to_string()],
            "http://rotated-refresh.test/",
        )
        .await
        .unwrap();

        assert_eq!(token.access_token, "new-access");
        assert_eq!(
            token.refresh_token.as_deref(),
            Some("rotated-refresh"),
            "rotated refresh_token from server must be used"
        );

        let _ = server.await;
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn do_refresh_returns_error_on_server_failure() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("AGC_CONFIG_DIR", dir.path());

        let (server_url, server) =
            spawn_token_server(400, r#"{"error":"invalid_grant"}"#).await;

        let result = do_refresh(
            &server_url,
            "client-id",
            "bad-refresh",
            &[],
            "http://do-refresh-fail.test/",
        )
        .await;

        assert!(result.is_err(), "do_refresh must return Err on HTTP 4xx");

        let _ = server.await;
    }
}

/// Mask a token for display: show first 4 and last 4 chars, hide the rest.
/// Uses char-based indexing so multi-byte UTF-8 tokens never cause a panic.
/// Strings of 8 chars or fewer are fully replaced with `****`.
fn mask_token(s: &str) -> String {
    let n = s.chars().count();
    if n > 8 {
        let prefix: String = s.chars().take(4).collect();
        let suffix: String = s.chars().skip(n - 4).collect();
        format!("{prefix}****{suffix}")
    } else {
        "****".to_string()
    }
}

fn urlenccode(s: &str) -> String {
    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}
