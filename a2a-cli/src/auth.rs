//! OAuth2 flows for A2A agents.
//!
//! Parses security schemes from raw card JSON (handles both v0.3 and v1 formats).
//! Supports: Authorization Code + PKCE, Device Code, Client Credentials, Bearer.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use oauth2::basic::BasicClient;
use oauth2::reqwest as oauth2_reqwest;
use oauth2::{
    AuthUrl, ClientId, ClientSecret, DeviceAuthorizationUrl, Scope,
    StandardDeviceAuthorizationResponse, TokenUrl,
};
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::config::Agent;
use crate::error::{A2aCliError, Result};
use crate::token_store::{Token, TokenGrantType, delete_token, load_token, save_token};
use crate::validate::validate_oauth_client_id;

// ── Extracted OAuth flow info (protocol-version agnostic) ─────────────

#[derive(Debug, Clone)]
pub enum OAuthFlow {
    AuthorizationCode {
        auth_url: String,
        token_url: String,
        scopes: Vec<String>,
    },
    DeviceCode {
        device_auth_url: String,
        token_url: String,
        scopes: Vec<String>,
    },
    ClientCredentials {
        token_url: String,
        scopes: Vec<String>,
    },
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
        let flows_val = scheme
            .get("oauth2SecurityScheme")
            .and_then(|o| o.get("flows"))
            // v0.3 format: { "type": "oauth2", "flows": { ... } }
            .or_else(|| {
                if scheme.get("type").and_then(|t| t.as_str()) == Some("oauth2") {
                    scheme.get("flows")
                } else {
                    None
                }
            });

        let Some(flows_obj) = flows_val.and_then(|f| f.as_object()) else {
            continue;
        };

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
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => vec![],
    }
}

/// Resolve the OAuth client ID using CLI override > environment > agent config.
///
/// Returns `Ok(None)` when no client ID is configured. Callers that are about
/// to start an OAuth flow should convert that to a user-facing auth error.
pub fn resolve_oauth_client_id(
    agent: &Agent,
    override_client_id: Option<&str>,
) -> Result<Option<String>> {
    if let Some(client_id) = override_client_id {
        validate_oauth_client_id(client_id)?;
        return Ok(Some(client_id.to_string()));
    }

    if let Ok(client_id) = std::env::var("A2A_CLIENT_ID")
        && !client_id.is_empty()
    {
        validate_oauth_client_id(&client_id)?;
        return Ok(Some(client_id));
    }

    let client_id = &agent.oauth_or_default().client_id;
    if client_id.is_empty() {
        return Ok(None);
    }
    validate_oauth_client_id(client_id)?;
    Ok(Some(client_id.clone()))
}

fn require_oauth_client_id(agent: &Agent, override_client_id: Option<&str>) -> Result<String> {
    resolve_oauth_client_id(agent, override_client_id)?.ok_or_else(|| {
        A2aCliError::Auth(
            "OAuth client ID is required. Pass `a2a auth login --client-id <id>`, set A2A_CLIENT_ID, or save it with `a2a agent update <alias> --client-id <id>`."
                .to_string(),
        )
    })
}

// ── Public API ────────────────────────────────────────────────────────

pub async fn login(
    agent_url: &str,
    agent: &Agent,
    card: &Value,
    client_id_override: Option<&str>,
) -> Result<Option<String>> {
    if let Some(client_id) = client_id_override {
        validate_oauth_client_id(client_id)?;
    }

    if let Some(token) = load_token(agent_url)?
        && !token.is_expired()
    {
        return Ok(Some(token.access_token));
    }

    let flows = extract_oauth_flows(card);
    if flows.is_empty() {
        return Ok(None);
    }
    let client_id = require_oauth_client_id(agent, client_id_override)?;

    // Prefer auth code, then device code, then client credentials — use first declared flow.
    let token = match flows.first() {
        Some(OAuthFlow::AuthorizationCode {
            auth_url,
            token_url,
            scopes,
        }) => {
            let cfg_scopes = if agent.oauth_or_default().scopes.is_empty() {
                scopes
            } else {
                &agent.oauth_or_default().scopes
            };
            auth_code_pkce_flow(auth_url, token_url, &client_id, cfg_scopes, agent_url).await?
        }
        Some(OAuthFlow::DeviceCode {
            device_auth_url,
            token_url,
            scopes,
        }) => {
            let cfg_scopes = if agent.oauth_or_default().scopes.is_empty() {
                scopes
            } else {
                &agent.oauth_or_default().scopes
            };
            device_code_flow(
                device_auth_url,
                token_url,
                &client_id,
                cfg_scopes,
                agent_url,
            )
            .await?
        }
        Some(OAuthFlow::ClientCredentials { token_url, scopes }) => {
            let cfg_scopes = if agent.oauth_or_default().scopes.is_empty() {
                scopes
            } else {
                &agent.oauth_or_default().scopes
            };
            client_credentials_flow(token_url, &client_id, cfg_scopes, agent_url).await?
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
/// - Expired client-credentials token → request a new access token, save, return it.
/// - Expired but no refresh capability → return an auth error instead of retrying unauthenticated.
pub async fn refresh_if_expired(agent_url: &str, client_id: &str) -> Result<Option<String>> {
    let Some(token) = load_token(agent_url)? else {
        return Ok(None);
    };
    if !token.is_expired() {
        return Ok(Some(token.access_token));
    }
    refresh_expired_token(agent_url, token, client_id)
        .await
        .map(Some)
}

async fn refresh_expired_token(agent_url: &str, token: Token, client_id: &str) -> Result<String> {
    let client_id = if client_id.trim().is_empty() {
        token.client_id.clone().unwrap_or_default()
    } else {
        client_id.to_string()
    };
    if client_id.trim().is_empty() {
        return Err(A2aCliError::Auth(
            "stored token is expired and OAuth client ID is unavailable; run `a2a auth login --client-id <id>` to re-authenticate".to_string(),
        ));
    }
    validate_oauth_client_id(&client_id)?;
    let Some(token_url) = token.token_url.as_deref() else {
        return Err(A2aCliError::Auth(
            "stored token is expired and token endpoint URL is unavailable; run `a2a auth login` to re-authenticate".to_string(),
        ));
    };

    if let Some(refresh_token) = token.refresh_token.as_deref() {
        let new_token = do_refresh(
            token_url,
            &client_id,
            refresh_token,
            &token.scopes,
            agent_url,
            token.grant_type,
        )
        .await?;
        return Ok(new_token.access_token);
    }

    // Client Credentials grants normally do not issue refresh tokens. Renew by
    // asking the token endpoint for a fresh access token. Legacy tokens do not
    // have grant_type stored, so allow that ambiguous case to try this path too.
    let can_try_client_credentials =
        token.grant_type == Some(TokenGrantType::ClientCredentials) || token.grant_type.is_none();
    if !can_try_client_credentials {
        return Err(A2aCliError::Auth(
            "stored token is expired and has no refresh token; run `a2a auth login` to re-authenticate".to_string(),
        ));
    }
    if std::env::var("A2A_CLIENT_SECRET")
        .ok()
        .filter(|s| !s.is_empty())
        .is_none()
    {
        return Err(A2aCliError::Auth(
            "stored token is expired and has no refresh token. If this agent uses Client Credentials, set A2A_CLIENT_SECRET; otherwise run `a2a auth login` to re-authenticate".to_string(),
        ));
    }

    let new_token =
        client_credentials_flow(token_url, &client_id, &token.scopes, agent_url).await?;
    Ok(new_token.access_token)
}

async fn do_refresh(
    token_url: &str,
    client_id: &str,
    refresh_token: &str,
    scopes: &[String],
    agent_url: &str,
    grant_type: Option<TokenGrantType>,
) -> Result<Token> {
    let http = reqwest::Client::new();
    let resp = http
        .post(token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
        ])
        .send()
        .await
        .map_err(A2aCliError::Http)?;
    if !resp.status().is_success() {
        return Err(A2aCliError::Auth(format!(
            "token refresh returned HTTP {}",
            resp.status()
        )));
    }
    let body: Value = resp.json().await.map_err(A2aCliError::Http)?;
    // Preserve the existing refresh_token if the server doesn't issue a new one.
    let mut token = token_from_json(&body, scopes, Some(token_url), Some(client_id), grant_type)?;
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
        None => Ok(TokenStatus {
            authenticated: false,
            expired: false,
            expires_at: None,
            scopes: vec![],
            masked_token: None,
        }),
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

// ── Browser open ─────────────────────────────────────────────────────

/// Open `url` in the system browser.  Always returns `false` in test builds
/// so tests never actually launch a browser.
fn open_browser(url: &str) -> bool {
    #[cfg(test)]
    {
        let _ = url;
        return false; // treat as "browser did not open" — prints fallback URL
    }
    #[cfg_attr(test, allow(unreachable_code))]
    open::that(url).is_ok()
}

// ── Test hook: capture the auth URL before opening the browser ────────

#[cfg(test)]
static AUTH_URL_CAPTURE: std::sync::OnceLock<
    std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
fn auth_url_capture()
-> &'static std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>> {
    AUTH_URL_CAPTURE.get_or_init(|| std::sync::Mutex::new(None))
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
        .map_err(|e| A2aCliError::Auth(format!("bind local server: {e}")))?;
    let port = listener.local_addr().map_err(A2aCliError::Io)?.port();
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
    if !scope_str.is_empty() {
        params.push(("scope", &scope_str));
    }
    params.push(("code_challenge", &code_challenge));
    params.push(("state", &state));

    let full_auth_url = format!(
        "{}?{}",
        auth_url,
        params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlenccode(v)))
            .collect::<Vec<_>>()
            .join("&")
    );

    // Capture auth URL in tests before opening the browser.
    #[cfg(test)]
    if let Ok(guard) = auth_url_capture().lock() {
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(full_auth_url.clone());
        }
    }

    if open_browser(&full_auth_url) {
        eprintln!("\nOpening browser for authentication...");
        eprintln!("If the browser did not open, visit:\n\n  {full_auth_url}\n");
    } else {
        eprintln!("\nTo authenticate, open this URL in your browser:\n\n  {full_auth_url}\n");
    }

    // Wait for the callback.
    let (code, returned_state) = wait_for_callback(listener).await?;

    if returned_state != state {
        return Err(A2aCliError::Auth(
            "OAuth state mismatch — possible CSRF".to_string(),
        ));
    }

    // Exchange code for token.
    let http = reqwest::Client::new();
    let resp = http
        .post(token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect_uri),
            ("client_id", client_id),
            ("code_verifier", &code_verifier),
        ])
        .send()
        .await
        .map_err(A2aCliError::Http)?;

    let body: Value = resp.json().await.map_err(A2aCliError::Http)?;
    let token = token_from_json(
        &body,
        scopes,
        Some(token_url),
        Some(client_id),
        Some(TokenGrantType::AuthorizationCode),
    )?;
    save_token(agent_url, &token)?;
    Ok(token)
}

async fn wait_for_callback(listener: tokio::net::TcpListener) -> Result<(String, String)> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    eprintln!("Waiting for browser redirect...\n");

    let (mut stream, _) = listener
        .accept()
        .await
        .map_err(|e| A2aCliError::Auth(format!("accept callback: {e}")))?;

    let mut reader = BufReader::new(&mut stream);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .await
        .map_err(|e| A2aCliError::Auth(format!("read callback: {e}")))?;

    // Parse GET /callback?code=X&state=Y HTTP/1.1
    let path = request_line.split_whitespace().nth(1).unwrap_or("");
    let query = path.split('?').nth(1).unwrap_or("");
    let params: HashMap<_, _> = query
        .split('&')
        .filter_map(|p| {
            let mut s = p.splitn(2, '=');
            Some((s.next()?, s.next()?))
        })
        .collect();

    let code = params
        .get("code")
        .ok_or_else(|| A2aCliError::Auth("no code in callback".to_string()))?
        .to_string();
    let state = params.get("state").copied().unwrap_or("").to_string();

    // Send success response to browser.
    let body = "<html><head><meta charset=\"utf-8\"></head><body><h2>Authentication successful - you can close this tab.</h2></body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes()).await; // best-effort: auth code already captured, browser UX only

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
    let http_client = oauth2_reqwest::ClientBuilder::new()
        .redirect(oauth2_reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| A2aCliError::Auth(format!("build http client: {e}")))?;

    let client = BasicClient::new(ClientId::new(client_id.to_string()))
        .set_auth_uri(
            AuthUrl::new("https://placeholder.invalid/auth".to_string())
                .map_err(|e| A2aCliError::Auth(format!("auth URL: {e}")))?,
        )
        .set_token_uri(
            TokenUrl::new(token_url.to_string())
                .map_err(|e| A2aCliError::Auth(format!("token URL: {e}")))?,
        )
        .set_device_authorization_url(
            DeviceAuthorizationUrl::new(device_auth_url.to_string())
                .map_err(|e| A2aCliError::Auth(format!("device auth URL: {e}")))?,
        );

    let details: StandardDeviceAuthorizationResponse = client
        .exchange_device_code()
        .add_scopes(scopes.iter().map(|s| Scope::new(s.clone())))
        .request_async(&http_client)
        .await
        .map_err(|e| A2aCliError::Auth(format!("device authorization: {e}")))?;

    eprintln!(
        "\nTo authenticate, visit: {}",
        details.verification_uri().as_str()
    );
    eprintln!("Enter code: {}\n", details.user_code().secret());

    let token_response = client
        .exchange_device_access_token(&details)
        .request_async(&http_client, tokio::time::sleep, None)
        .await
        .map_err(|e| A2aCliError::Auth(format!("device token exchange: {e}")))?;

    use oauth2::TokenResponse;
    let token = Token {
        access_token: token_response.access_token().secret().clone(),
        refresh_token: token_response.refresh_token().map(|t| t.secret().clone()),
        expires_at: token_response
            .expires_in()
            .map(|d| unix_now() + d.as_secs()),
        token_type: "Bearer".to_string(),
        scopes: scopes.to_vec(),
        token_url: Some(token_url.to_string()),
        client_id: Some(client_id.to_string()),
        grant_type: Some(TokenGrantType::DeviceCode),
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
    let secret = std::env::var("A2A_CLIENT_SECRET")
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            A2aCliError::Auth("client credentials requires A2A_CLIENT_SECRET".to_string())
        })?;

    let http_client = oauth2_reqwest::ClientBuilder::new()
        .redirect(oauth2_reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| A2aCliError::Auth(format!("build http client: {e}")))?;

    let client = BasicClient::new(ClientId::new(client_id.to_string()))
        .set_client_secret(ClientSecret::new(secret))
        .set_auth_uri(
            AuthUrl::new("https://placeholder.invalid/auth".to_string())
                .map_err(|e| A2aCliError::Auth(format!("auth URL: {e}")))?,
        )
        .set_token_uri(
            TokenUrl::new(token_url.to_string())
                .map_err(|e| A2aCliError::Auth(format!("token URL: {e}")))?,
        );

    use oauth2::TokenResponse;
    let resp = client
        .exchange_client_credentials()
        .add_scopes(scopes.iter().map(|s| Scope::new(s.clone())))
        .request_async(&http_client)
        .await
        .map_err(|e| A2aCliError::Auth(format!("client credentials: {e}")))?;

    let token = Token {
        access_token: resp.access_token().secret().clone(),
        refresh_token: resp.refresh_token().map(|t| t.secret().clone()),
        expires_at: resp.expires_in().map(|d| unix_now() + d.as_secs()),
        token_type: "Bearer".to_string(),
        scopes: scopes.to_vec(),
        token_url: Some(token_url.to_string()),
        client_id: Some(client_id.to_string()),
        grant_type: Some(TokenGrantType::ClientCredentials),
    };
    save_token(agent_url, &token)?;
    Ok(token)
}

// ── Helpers ───────────────────────────────────────────────────────────

fn token_from_json(
    body: &Value,
    scopes: &[String],
    token_url: Option<&str>,
    client_id: Option<&str>,
    grant_type: Option<TokenGrantType>,
) -> Result<Token> {
    let access_token = body
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| A2aCliError::Auth(format!("no access_token in response: {body}")))?
        .to_string();

    let expires_at = body
        .get("expires_in")
        .and_then(|v| v.as_u64())
        .map(|secs| unix_now() + secs);

    Ok(Token {
        access_token,
        refresh_token: body
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        expires_at,
        token_type: body
            .get("token_type")
            .and_then(|v| v.as_str())
            .unwrap_or("Bearer")
            .to_string(),
        scopes: scopes.to_vec(),
        token_url: token_url.map(str::to_string),
        client_id: client_id
            .filter(|id| !id.trim().is_empty())
            .map(str::to_string),
        grant_type,
    })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── Shared test helpers ───────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod test_utils {
    /// RAII guard: saves an env var on creation and restores it on drop.
    pub struct EnvGuard {
        name: &'static str,
        original: Option<std::ffi::OsString>,
    }
    impl EnvGuard {
        pub fn set(name: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let original = std::env::var_os(name);
            // SAFETY: tests using EnvGuard are annotated #[serial_test::serial],
            // which ensures no concurrent env mutations within this test suite.
            unsafe { std::env::set_var(name, value) };
            Self { name, original }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                // SAFETY: see EnvGuard::set — serial test execution prevents races.
                Some(v) => unsafe { std::env::set_var(self.name, v) },
                None => unsafe { std::env::remove_var(self.name) },
            }
        }
    }

    /// Spawn a minimal HTTP server that handles one POST and responds with
    /// `status` + JSON `body`.
    pub async fn spawn_token_server(
        status: u16,
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{port}/token");

        let handle = tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let (read_half, mut write_half) = tokio::io::split(stream);
            let mut reader = BufReader::new(read_half);
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
                if let Some(v) = line.to_lowercase().strip_prefix("content-length: ") {
                    content_length = v.trim().parse().unwrap_or(0);
                }
            }
            if content_length > 0 {
                let mut buf = vec![0u8; content_length];
                let _ = reader.read_exact(&mut buf).await;
            }
            let status_line = if status == 200 {
                "200 OK"
            } else {
                "400 Bad Request"
            };
            let resp = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = write_half.write_all(resp.as_bytes()).await;
        });
        (url, handle)
    }
}

#[cfg(test)]
mod tests {
    use super::test_utils::EnvGuard;
    use super::*;
    use crate::config::OAuthConfig;
    use serial_test::serial;

    fn agent_with_client_id(client_id: Option<&str>) -> Agent {
        Agent {
            url: "http://agent.test/".to_string(),
            description: String::new(),
            transport: String::new(),
            oauth: client_id.map(|id| OAuthConfig {
                client_id: id.to_string(),
                scopes: vec![],
            }),
        }
    }

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
            OAuthFlow::AuthorizationCode {
                auth_url,
                token_url,
                scopes,
            } => {
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
            OAuthFlow::DeviceCode {
                device_auth_url,
                token_url,
                scopes,
            } => {
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

    #[test]
    fn extract_unsupported_implicit_and_password_flows_returns_empty() {
        let card = serde_json::json!({
            "securitySchemes": {
                "myOAuth": {
                    "oauth2SecurityScheme": {
                        "flows": {
                            "implicit": {
                                "authorizationUrl": "https://auth.example.com/authorize",
                                "scopes": {}
                            },
                            "password": {
                                "tokenUrl": "https://auth.example.com/token",
                                "scopes": {}
                            }
                        }
                    }
                }
            }
        });

        assert!(extract_oauth_flows(&card).is_empty());
    }

    // ── client ID resolution ─────────────────────────────────────────

    #[test]
    fn resolve_client_id_prefers_override() {
        let agent = agent_with_client_id(Some("configured-client"));

        let client_id = resolve_oauth_client_id(&agent, Some("override-client")).unwrap();

        assert_eq!(client_id.as_deref(), Some("override-client"));
    }

    #[test]
    #[serial]
    fn resolve_client_id_uses_env_before_config() {
        let _env = EnvGuard::set("A2A_CLIENT_ID", "env-client");
        let agent = agent_with_client_id(Some("configured-client"));

        let client_id = resolve_oauth_client_id(&agent, None).unwrap();

        assert_eq!(client_id.as_deref(), Some("env-client"));
    }

    #[test]
    #[serial]
    fn resolve_client_id_uses_config_when_no_override_or_env() {
        let _env = EnvGuard::set("A2A_CLIENT_ID", "");
        let agent = agent_with_client_id(Some("configured-client"));

        let client_id = resolve_oauth_client_id(&agent, None).unwrap();

        assert_eq!(client_id.as_deref(), Some("configured-client"));
    }

    #[test]
    #[serial]
    fn resolve_client_id_accepts_cimd_http_url() {
        let _env = EnvGuard::set("A2A_CLIENT_ID", "");
        let agent = agent_with_client_id(Some("http://cimd.example.com/clients/a2a-cli"));

        let client_id = resolve_oauth_client_id(&agent, None).unwrap();

        assert_eq!(
            client_id.as_deref(),
            Some("http://cimd.example.com/clients/a2a-cli")
        );
    }

    #[test]
    #[serial]
    fn resolve_client_id_returns_none_when_missing() {
        let _env = EnvGuard::set("A2A_CLIENT_ID", "");
        let agent = agent_with_client_id(None);

        let client_id = resolve_oauth_client_id(&agent, None).unwrap();

        assert!(client_id.is_none());
    }

    #[test]
    fn resolve_client_id_rejects_empty_override() {
        let agent = agent_with_client_id(Some("configured-client"));

        let result = resolve_oauth_client_id(&agent, Some(""));

        assert!(result.is_err());
    }

    // ── login preflight ──────────────────────────────────────────────

    #[tokio::test]
    #[serial]
    async fn login_requires_client_id_for_oauth_card() {
        let dir = tempfile::tempdir().unwrap();
        let _cfg = EnvGuard::set("A2A_CONFIG_DIR", dir.path());
        let _client_id = EnvGuard::set("A2A_CLIENT_ID", "");
        let agent = agent_with_client_id(None);

        let err = login(
            "http://missing-client.test/",
            &agent,
            &card_with_client_credentials(),
            None,
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("OAuth client ID is required"));
    }

    #[tokio::test]
    #[serial]
    async fn login_without_oauth_flows_does_not_require_client_id() {
        let dir = tempfile::tempdir().unwrap();
        let _cfg = EnvGuard::set("A2A_CONFIG_DIR", dir.path());
        let _client_id = EnvGuard::set("A2A_CLIENT_ID", "");
        let agent = agent_with_client_id(None);

        let result = login(
            "http://no-auth-client.test/",
            &agent,
            &serde_json::json!({"name": "No Auth"}),
            None,
        )
        .await
        .unwrap();

        assert!(result.is_none());
    }
}

#[cfg(test)]
mod oauth_flow_tests {
    use super::test_utils::EnvGuard;
    use super::*;
    use crate::config::OAuthConfig;
    use crate::token_store::load_token;
    use base64::Engine;
    use percent_encoding::percent_decode_str;
    use serial_test::serial;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    #[derive(Debug, Clone)]
    struct CapturedRequest {
        path: String,
        headers: HashMap<String, String>,
        body: String,
    }

    impl CapturedRequest {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .get(&name.to_ascii_lowercase())
                .map(String::as_str)
        }
    }

    fn agent_with_client_id(client_id: Option<&str>) -> Agent {
        Agent {
            url: "http://agent.test/".to_string(),
            description: String::new(),
            transport: String::new(),
            oauth: client_id.map(|id| OAuthConfig {
                client_id: id.to_string(),
                scopes: vec![],
            }),
        }
    }

    fn decode_form_component(value: &str) -> String {
        let value = value.replace('+', " ");
        percent_decode_str(&value).decode_utf8_lossy().into_owned()
    }

    fn form_body(body: &str) -> HashMap<String, String> {
        body.split('&')
            .filter_map(|pair| {
                let (key, value) = pair.split_once('=')?;
                Some((decode_form_component(key), decode_form_component(value)))
            })
            .collect()
    }

    fn device_code_card(device_url: &str, token_url: &str) -> Value {
        serde_json::json!({
            "securitySchemes": {
                "myOAuth": {
                    "oauth2SecurityScheme": {
                        "flows": {
                            "deviceCode": {
                                "deviceAuthorizationUrl": device_url,
                                "tokenUrl": token_url,
                                "scopes": { "openid": "OpenID Connect", "email": "Email" }
                            }
                        }
                    }
                }
            }
        })
    }

    fn client_credentials_card(token_url: &str) -> Value {
        serde_json::json!({
            "securitySchemes": {
                "myOAuth": {
                    "oauth2SecurityScheme": {
                        "flows": {
                            "clientCredentials": {
                                "tokenUrl": token_url,
                                "scopes": { "api": "API access" }
                            }
                        }
                    }
                }
            }
        })
    }

    async fn spawn_oauth_flow_server_with_responses(
        expected_requests: usize,
    ) -> (
        String,
        Arc<Mutex<Vec<CapturedRequest>>>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");
        let captured = Arc::new(Mutex::new(Vec::new()));
        let captured_for_task = Arc::clone(&captured);

        let handle = tokio::spawn(async move {
            for _ in 0..expected_requests {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let (read_half, mut write_half) = tokio::io::split(stream);
                let mut reader = BufReader::new(read_half);
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).await.unwrap_or(0) == 0 {
                    continue;
                }
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .to_string();
                let mut headers = HashMap::new();
                let mut content_length = 0usize;
                let mut line = String::new();
                loop {
                    line.clear();
                    if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                        break;
                    }
                    let trimmed = line.trim_end();
                    if trimmed.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = trimmed.split_once(':') {
                        let name = name.trim().to_ascii_lowercase();
                        let value = value.trim().to_string();
                        if name == "content-length" {
                            content_length = value.parse().unwrap_or(0);
                        }
                        headers.insert(name, value);
                    }
                }
                let mut body_bytes = vec![0u8; content_length];
                if content_length > 0 {
                    let _ = reader.read_exact(&mut body_bytes).await;
                }
                captured_for_task.lock().unwrap().push(CapturedRequest {
                    path: path.clone(),
                    headers,
                    body: String::from_utf8_lossy(&body_bytes).into_owned(),
                });

                let body = match path.as_str() {
                    "/device" => {
                        r#"{"device_code":"mock-device-code","user_code":"ABCD-EFGH","verification_uri":"http://verify.example.com","expires_in":600,"interval":1}"#
                    }
                    "/token" => {
                        r#"{"access_token":"flow-access-token","expires_in":3600,"token_type":"Bearer","refresh_token":"flow-refresh-token"}"#
                    }
                    _ => r#"{"error":"not_found"}"#,
                };
                let status_line = if path == "/device" || path == "/token" {
                    "200 OK"
                } else {
                    "404 Not Found"
                };
                let response = format!(
                    "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = write_half.write_all(response.as_bytes()).await;
            }
        });

        (base_url, captured, handle)
    }

    #[tokio::test]
    #[serial]
    async fn login_device_code_flow_exchanges_and_saves_token() {
        let dir = tempfile::tempdir().unwrap();
        let _cfg = EnvGuard::set("A2A_CONFIG_DIR", dir.path());
        let _client_id = EnvGuard::set("A2A_CLIENT_ID", "device-client");

        let (base_url, captured, server) = spawn_oauth_flow_server_with_responses(2).await;
        let card = device_code_card(&format!("{base_url}/device"), &format!("{base_url}/token"));

        let token = login(
            "http://device-agent.test/",
            &agent_with_client_id(None),
            &card,
            None,
        )
        .await
        .unwrap();

        assert_eq!(token.as_deref(), Some("flow-access-token"));
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("oauth mock server did not finish")
            .unwrap();

        let requests = captured.lock().unwrap().clone();
        let device_req = requests
            .iter()
            .find(|req| req.path == "/device")
            .expect("device authorization request missing");
        let device_form = form_body(&device_req.body);
        assert_eq!(
            device_form.get("client_id").map(String::as_str),
            Some("device-client")
        );
        let scopes = device_form
            .get("scope")
            .map(|scope| scope.split_whitespace().collect::<Vec<_>>())
            .unwrap_or_default();
        assert!(scopes.contains(&"openid"));
        assert!(scopes.contains(&"email"));

        let token_req = requests
            .iter()
            .find(|req| req.path == "/token")
            .expect("device token request missing");
        let token_form = form_body(&token_req.body);
        assert_eq!(
            token_form.get("grant_type").map(String::as_str),
            Some("urn:ietf:params:oauth:grant-type:device_code")
        );
        assert_eq!(
            token_form.get("device_code").map(String::as_str),
            Some("mock-device-code")
        );

        let stored = load_token("http://device-agent.test/")
            .unwrap()
            .expect("device code token should be saved");
        assert_eq!(stored.access_token, "flow-access-token");
        assert_eq!(stored.grant_type, Some(TokenGrantType::DeviceCode));
    }

    #[tokio::test]
    #[serial]
    async fn login_client_credentials_flow_exchanges_and_saves_token_with_cimd_client_id() {
        let dir = tempfile::tempdir().unwrap();
        let _cfg = EnvGuard::set("A2A_CONFIG_DIR", dir.path());
        let cimd_client_id = "http://cimd.example.com/clients/a2a-cli";
        let _client_id = EnvGuard::set("A2A_CLIENT_ID", cimd_client_id);
        let _secret = EnvGuard::set("A2A_CLIENT_SECRET", "client-secret");

        let (base_url, captured, server) = spawn_oauth_flow_server_with_responses(1).await;
        let card = client_credentials_card(&format!("{base_url}/token"));

        let token = login(
            "http://client-credentials-agent.test/",
            &agent_with_client_id(None),
            &card,
            None,
        )
        .await
        .unwrap();

        assert_eq!(token.as_deref(), Some("flow-access-token"));
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("oauth mock server did not finish")
            .unwrap();

        let requests = captured.lock().unwrap().clone();
        let token_req = requests
            .iter()
            .find(|req| req.path == "/token")
            .expect("client credentials token request missing");
        let token_form = form_body(&token_req.body);
        assert_eq!(
            token_form.get("grant_type").map(String::as_str),
            Some("client_credentials")
        );
        assert_eq!(token_form.get("scope").map(String::as_str), Some("api"));

        let client_id_in_request = token_form
            .get("client_id")
            .is_some_and(|client_id| client_id == cimd_client_id)
            || token_req.header("authorization").is_some_and(|auth| {
                let Some(encoded) = auth.strip_prefix("Basic ") else {
                    return false;
                };
                let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
                    return false;
                };
                decode_form_component(&String::from_utf8_lossy(&decoded)).contains(cimd_client_id)
            });
        assert!(
            client_id_in_request,
            "CIMD URL client_id must be sent to the token endpoint"
        );

        let stored = load_token("http://client-credentials-agent.test/")
            .unwrap()
            .expect("client credentials token should be saved");
        assert_eq!(stored.access_token, "flow-access-token");
        assert_eq!(stored.client_id.as_deref(), Some(cimd_client_id));
        assert_eq!(stored.grant_type, Some(TokenGrantType::ClientCredentials));
    }
}

#[cfg(test)]
mod refresh_tests {
    use super::test_utils::{EnvGuard, spawn_token_server};
    use super::*;
    use crate::token_store::{Token, load_token, save_token};
    use std::time::{SystemTime, UNIX_EPOCH};

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
            client_id: None,
            grant_type: Some(TokenGrantType::AuthorizationCode),
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
            client_id: None,
            grant_type: Some(TokenGrantType::AuthorizationCode),
        }
    }

    // ── refresh_if_expired ────────────────────────────────────────────

    #[tokio::test]
    #[serial_test::serial]
    async fn no_stored_token_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("A2A_CONFIG_DIR", dir.path());

        let result = refresh_if_expired("http://no-token.test/", "client-id").await;
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn valid_token_returned_without_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("A2A_CONFIG_DIR", dir.path());

        save_token("http://valid.test/", &valid_token()).unwrap();
        let result = refresh_if_expired("http://valid.test/", "client-id")
            .await
            .unwrap();
        assert_eq!(result, Some("valid-access".to_string()));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn expired_no_refresh_token_without_secret_returns_auth_error() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("A2A_CONFIG_DIR", dir.path());
        let _secret = EnvGuard::set("A2A_CLIENT_SECRET", "");

        save_token(
            "http://no-refresh.test/",
            &expired_token(None, Some("https://auth.example.com/token")),
        )
        .unwrap();

        let err = refresh_if_expired("http://no-refresh.test/", "client-id")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("has no refresh token"));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn expired_client_credentials_token_renews_without_refresh_token() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("A2A_CONFIG_DIR", dir.path());
        let _secret = EnvGuard::set("A2A_CLIENT_SECRET", "client-secret");

        let (server_url, server) = spawn_token_server(
            200,
            r#"{"access_token":"new-client-access","expires_in":3600,"token_type":"Bearer"}"#,
        )
        .await;

        let mut token = expired_token(None, Some(&server_url));
        token.grant_type = Some(TokenGrantType::ClientCredentials);
        save_token("http://client-credentials.test/", &token).unwrap();

        let result = refresh_if_expired("http://client-credentials.test/", "client-id")
            .await
            .unwrap();
        assert_eq!(result, Some("new-client-access".to_string()));

        let stored = load_token("http://client-credentials.test/")
            .unwrap()
            .expect("renewed token should be saved");
        assert_eq!(stored.access_token, "new-client-access");
        assert_eq!(stored.grant_type, Some(TokenGrantType::ClientCredentials));

        let _ = server.await;
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn expired_no_token_url_returns_auth_error() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("A2A_CONFIG_DIR", dir.path());

        save_token(
            "http://no-token-url.test/",
            &expired_token(Some("old-refresh"), None),
        )
        .unwrap();

        let err = refresh_if_expired("http://no-token-url.test/", "client-id")
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("token endpoint URL is unavailable")
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn expired_token_refreshes_successfully() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("A2A_CONFIG_DIR", dir.path());

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

        let result = refresh_if_expired("http://refresh-ok.test/", "client-id")
            .await
            .unwrap();
        assert_eq!(result, Some("new-access".to_string()));

        // Verify the new token was persisted.
        let stored = load_token("http://refresh-ok.test/").unwrap().unwrap();
        assert_eq!(stored.access_token, "new-access");
        assert_eq!(stored.refresh_token.as_deref(), Some("new-refresh"));
        assert_eq!(stored.grant_type, Some(TokenGrantType::AuthorizationCode));

        let _ = server.await;
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn expired_token_uses_stored_client_id_when_argument_empty() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("A2A_CONFIG_DIR", dir.path());

        let (server_url, server) = spawn_token_server(
            200,
            r#"{"access_token":"new-access","expires_in":3600,"token_type":"Bearer","refresh_token":"new-refresh"}"#,
        )
        .await;

        let mut token = expired_token(Some("old-refresh"), Some(&server_url));
        token.client_id = Some("stored-client-id".to_string());
        save_token("http://stored-client.test/", &token).unwrap();

        let result = refresh_if_expired("http://stored-client.test/", "")
            .await
            .unwrap();
        assert_eq!(result, Some("new-access".to_string()));

        let stored = load_token("http://stored-client.test/")
            .unwrap()
            .expect("refreshed token should be saved");
        assert_eq!(stored.client_id.as_deref(), Some("stored-client-id"));

        let _ = server.await;
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn expired_token_server_error_returns_auth_error() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("A2A_CONFIG_DIR", dir.path());

        let (server_url, server) = spawn_token_server(400, r#"{"error":"invalid_grant"}"#).await;

        save_token(
            "http://refresh-fail.test/",
            &expired_token(Some("bad-refresh"), Some(&server_url)),
        )
        .unwrap();

        let err = refresh_if_expired("http://refresh-fail.test/", "client-id")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("token refresh returned HTTP"));

        let _ = server.await;
    }

    // ── do_refresh ────────────────────────────────────────────────────

    #[tokio::test]
    #[serial_test::serial]
    async fn do_refresh_preserves_refresh_token_when_server_omits_it() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("A2A_CONFIG_DIR", dir.path());

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
            Some(TokenGrantType::AuthorizationCode),
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
        let _env = EnvGuard::set("A2A_CONFIG_DIR", dir.path());

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
            Some(TokenGrantType::AuthorizationCode),
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
        let _env = EnvGuard::set("A2A_CONFIG_DIR", dir.path());

        let (server_url, server) = spawn_token_server(400, r#"{"error":"invalid_grant"}"#).await;

        let result = do_refresh(
            &server_url,
            "client-id",
            "bad-refresh",
            &[],
            "http://do-refresh-fail.test/",
            Some(TokenGrantType::AuthorizationCode),
        )
        .await;

        assert!(result.is_err(), "do_refresh must return Err on HTTP 4xx");

        let _ = server.await;
    }
}

/// Integration test for the full PKCE authorization code flow.
#[cfg(test)]
mod pkce_flow_tests {
    use super::test_utils::{EnvGuard, spawn_token_server};
    use super::*;
    use crate::token_store::load_token;
    use serial_test::serial;
    use std::time::Duration;

    /// Simulate the browser redirect: connect to `callback_url` and send a GET
    /// with the given `code` and `state`, exactly as a browser would after the
    /// authorization server redirects the user.
    async fn send_browser_callback(callback_url: &str, code: &str, state: &str) {
        use tokio::io::AsyncWriteExt;

        // Parse port from http://127.0.0.1:{port}/callback
        let port: u16 = callback_url
            .split(':')
            .nth(2)
            .and_then(|s| s.split('/').next())
            .and_then(|s| s.parse().ok())
            .expect("parse callback port");

        let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .expect("connect to callback server");

        let request = format!(
            "GET /callback?code={code}&state={state} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(request.as_bytes())
            .await
            .expect("write callback request");
        // Drain the response so the server can close cleanly.
        let _ = tokio::io::copy(&mut stream, &mut tokio::io::sink()).await;
    }

    /// Parse and percent-decode a query-string value from a URL.
    fn query_param(url: &str, key: &str) -> Option<String> {
        use percent_encoding::percent_decode_str;
        let query = url.split('?').nth(1)?;
        query.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            if k == key {
                Some(percent_decode_str(v).decode_utf8_lossy().into_owned())
            } else {
                None
            }
        })
    }

    #[tokio::test]
    #[serial]
    async fn pkce_flow_builds_correct_auth_url_receives_callback_and_saves_token() {
        // ── 1. Isolated config dir ────────────────────────────────────
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("A2A_CONFIG_DIR", dir.path());

        // ── 2. Mock token endpoint ────────────────────────────────────
        let (token_url, _token_server) = spawn_token_server(
            200,
            r#"{"access_token":"mock-access-token","expires_in":3600,"token_type":"Bearer","refresh_token":"mock-refresh"}"#,
        )
        .await;

        // ── 3. Install auth-URL capture channel ───────────────────────
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        *auth_url_capture().lock().unwrap() = Some(tx);

        // ── 4. Run PKCE flow in background ────────────────────────────
        let agent_url = "http://mock-agent.test/";
        let token_url_clone = token_url.clone();
        let flow_handle = tokio::spawn(async move {
            auth_code_pkce_flow(
                "http://mock-auth.test/authorize",
                &token_url_clone,
                "test-client-id",
                &["openid".to_string(), "email".to_string()],
                agent_url,
            )
            .await
        });

        // ── 5. Receive the auth URL the flow would open in a browser ──
        let auth_url = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("auth URL not captured within 5 s")
            .expect("channel closed before URL was sent");

        // Verify auth URL structure
        assert!(
            auth_url.starts_with("http://mock-auth.test/authorize"),
            "wrong base: {auth_url}"
        );
        assert_eq!(
            query_param(&auth_url, "response_type").as_deref(),
            Some("code")
        );
        assert_eq!(
            query_param(&auth_url, "client_id").as_deref(),
            Some("test-client-id")
        );
        assert_eq!(
            query_param(&auth_url, "code_challenge_method").as_deref(),
            Some("S256")
        );
        let code_challenge =
            query_param(&auth_url, "code_challenge").expect("code_challenge missing");
        assert!(
            !code_challenge.is_empty(),
            "code_challenge must not be empty"
        );
        let redirect_uri_enc =
            query_param(&auth_url, "redirect_uri").expect("redirect_uri missing");
        let redirect_uri = percent_encoding::percent_decode_str(&redirect_uri_enc)
            .decode_utf8_lossy()
            .into_owned();
        assert!(
            redirect_uri.starts_with("http://127.0.0.1:"),
            "redirect_uri must be localhost: {redirect_uri}"
        );
        let state = query_param(&auth_url, "state").expect("state missing");
        assert!(!state.is_empty(), "state must not be empty");

        // ── 6. Simulate browser sending the OAuth callback ────────────
        send_browser_callback(&redirect_uri, "mock-auth-code", &state).await;

        // ── 7. Wait for flow to complete ──────────────────────────────
        let token = flow_handle
            .await
            .expect("flow task panicked")
            .expect("auth_code_pkce_flow returned Err");

        assert_eq!(token.access_token, "mock-access-token");
        assert_eq!(token.refresh_token.as_deref(), Some("mock-refresh"));

        // ── 8. Verify token is persisted (encrypted at rest) ─────────
        let stored = load_token(agent_url)
            .expect("load_token failed")
            .expect("token not found after login");
        assert_eq!(stored.access_token, "mock-access-token");
        assert_eq!(stored.client_id.as_deref(), Some("test-client-id"));
        assert!(stored.expires_at.is_some(), "expires_at must be set");

        // ── Cleanup ───────────────────────────────────────────────────
        *auth_url_capture().lock().unwrap() = None;
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
