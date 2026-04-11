//! OAuth2 flows for A2A agents.
//!
//! Parses security schemes from raw card JSON (handles both v0.3 and v1 formats).
//! Supports: Authorization Code + PKCE, Device Code, Client Credentials, Bearer.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use oauth2::basic::BasicClient;
use oauth2::{
    AuthUrl, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge, RedirectUrl, Scope,
    TokenUrl, AuthorizationCode,
    devicecode::StandardDeviceAuthorizationResponse,
    DeviceAuthorizationUrl,
};
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::config::{Agent, OAuthConfig};
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
        if let Some(ac) = flows_obj.get("authorizationCode") {
            if let (Some(auth_url), Some(token_url)) = (
                ac.get("authorizationUrl").and_then(|v| v.as_str()),
                ac.get("tokenUrl").and_then(|v| v.as_str()),
            ) {
                let scopes = extract_scope_names(ac.get("scopes"));
                flows.push(OAuthFlow::AuthorizationCode {
                    auth_url: auth_url.to_string(),
                    token_url: token_url.to_string(),
                    scopes,
                });
            }
        }

        // Device Code
        if let Some(dc) = flows_obj.get("deviceCode") {
            if let (Some(device_url), Some(token_url)) = (
                dc.get("deviceAuthorizationUrl").and_then(|v| v.as_str()),
                dc.get("tokenUrl").and_then(|v| v.as_str()),
            ) {
                let scopes = extract_scope_names(dc.get("scopes"));
                flows.push(OAuthFlow::DeviceCode {
                    device_auth_url: device_url.to_string(),
                    token_url: token_url.to_string(),
                    scopes,
                });
            }
        }

        // Client Credentials
        if let Some(cc) = flows_obj.get("clientCredentials") {
            if let Some(token_url) = cc.get("tokenUrl").and_then(|v| v.as_str()) {
                let scopes = extract_scope_names(cc.get("scopes"));
                flows.push(OAuthFlow::ClientCredentials {
                    token_url: token_url.to_string(),
                    scopes,
                });
            }
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
    if let Some(token) = load_token(agent_url)? {
        if !token.is_expired() {
            return Ok(Some(token.access_token));
        }
    }

    let flows = extract_oauth_flows(card);
    if flows.is_empty() {
        return Ok(None);
    }

    // Prefer auth code, then device code, then client credentials.
    for flow in &flows {
        match flow {
            OAuthFlow::AuthorizationCode { auth_url, token_url, scopes } => {
                let cfg_scopes = if agent.oauth.scopes.is_empty() { scopes } else { &agent.oauth.scopes };
                let token = auth_code_pkce_flow(
                    auth_url, token_url, &agent.oauth.client_id, cfg_scopes, agent_url,
                ).await?;
                return Ok(Some(token.access_token));
            }
            OAuthFlow::DeviceCode { device_auth_url, token_url, scopes } => {
                let cfg_scopes = if agent.oauth.scopes.is_empty() { scopes } else { &agent.oauth.scopes };
                let token = device_code_flow(
                    device_auth_url, token_url, &agent.oauth.client_id, cfg_scopes, agent_url,
                ).await?;
                return Ok(Some(token.access_token));
            }
            OAuthFlow::ClientCredentials { token_url, scopes } => {
                let cfg_scopes = if agent.oauth.scopes.is_empty() { scopes } else { &agent.oauth.scopes };
                let token = client_credentials_flow(
                    token_url, &agent.oauth, agent_url,
                ).await?;
                return Ok(Some(token.access_token));
            }
        }
    }
    Ok(None)
}

pub fn logout(agent_url: &str) -> Result<()> {
    delete_token(agent_url)
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
            let masked = if t.access_token.len() > 8 {
                Some(format!("{}****{}", &t.access_token[..4], &t.access_token[t.access_token.len()-4..]))
            } else { Some("****".to_string()) };
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
    let token = token_from_json(&body, scopes)?;
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
    };
    save_token(agent_url, &token)?;
    Ok(token)
}

// ── Client Credentials ────────────────────────────────────────────────

async fn client_credentials_flow(
    token_url: &str,
    oauth_cfg: &OAuthConfig,
    agent_url: &str,
) -> Result<Token> {
    let secret = std::env::var("AGC_CLIENT_SECRET")
        .map_err(|_| AgcError::Auth("client credentials requires AGC_CLIENT_SECRET".to_string()))?;

    let client = BasicClient::new(
        ClientId::new(oauth_cfg.client_id.clone()),
        Some(ClientSecret::new(secret)),
        AuthUrl::new("https://placeholder.invalid/auth".to_string())
            .map_err(|e| AgcError::Auth(format!("auth URL: {e}")))?,
        Some(TokenUrl::new(token_url.to_string())
            .map_err(|e| AgcError::Auth(format!("token URL: {e}")))?),
    );

    use oauth2::TokenResponse;
    let resp = client
        .exchange_client_credentials()
        .add_scopes(oauth_cfg.scopes.iter().map(|s| Scope::new(s.clone())))
        .request_async(oauth2::reqwest::async_http_client)
        .await
        .map_err(|e| AgcError::Auth(format!("client credentials: {e}")))?;

    let token = Token {
        access_token: resp.access_token().secret().clone(),
        refresh_token: resp.refresh_token().map(|t| t.secret().clone()),
        expires_at: resp.expires_in().map(|d| unix_now() + d.as_secs()),
        token_type: "Bearer".to_string(),
        scopes: oauth_cfg.scopes.clone(),
    };
    save_token(agent_url, &token)?;
    Ok(token)
}

// ── Helpers ───────────────────────────────────────────────────────────

fn token_from_json(body: &Value, scopes: &[String]) -> Result<Token> {
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
    })
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn urlenccode(s: &str) -> String {
    s.chars().map(|c| match c {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
        _ => format!("%{:02X}", c as u32),
    }).collect()
}
