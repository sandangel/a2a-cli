//! A2A v0.3 backward-compatibility layer.
//!
//! The v0.3 protocol uses a flat JSON-RPC wire format with different method names
//! and message shapes compared to v1.0.  This crate provides:
//!
//! - [`is_v03`] — detect a v0.3 agent card
//! - [`transport_from_card`] — detect the preferred transport (JSON-RPC or REST)
//! - [`rpc_url_from_card`] — extract the endpoint URL from a v0.3 card
//! - [`normalize_card`] — up-convert a v0.3 card JSON into a v1 [`AgentCard`]
//! - [`Client`] — JSON-RPC or REST client for v0.3 agents
//! - [`MessageParams`] — parameters for send / stream operations

use futures::StreamExt;
use serde_json::Value;
use thiserror::Error;
use tokio::io::AsyncBufReadExt;
use tokio_util::io::StreamReader;

pub use a2a::AgentCard;

// ── Error types ───────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum V03Error {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("rpc error: {0}")]
    Rpc(String),

    #[error("card parse error: {0}")]
    CardParse(String),

    #[error("{0}")]
    Unsupported(&'static str),
}

/// Error returned by [`Client::stream_message`] and [`Client::subscribe`].
///
/// Separates v0.3 protocol errors from errors raised inside the caller's event
/// callback so callers can handle them independently.
#[derive(Debug)]
pub enum SseError<E> {
    Protocol(V03Error),
    Callback(E),
}

impl<E: std::fmt::Display> std::fmt::Display for SseError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Protocol(e) => write!(f, "{e}"),
            Self::Callback(e) => write!(f, "{e}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for SseError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Protocol(e) => Some(e),
            Self::Callback(e) => Some(e),
        }
    }
}

// ── Transport ─────────────────────────────────────────────────────────

/// Wire transport for v0.3 agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transport {
    /// JSON-RPC: all requests POST to a single RPC endpoint (default).
    #[default]
    JsonRpc,
    /// HTTP+JSON: RESTful paths with snake_case JSON bodies and responses.
    Rest,
}

/// Detect the preferred transport from a raw v0.3 agent card.
///
/// Returns [`Transport::Rest`] when `preferredTransport` is `"HTTP+JSON"`;
/// defaults to [`Transport::JsonRpc`] otherwise.
pub fn transport_from_card(raw: &Value) -> Transport {
    match raw.get("preferredTransport").and_then(|t| t.as_str()) {
        Some("HTTP+JSON") => Transport::Rest,
        _ => Transport::JsonRpc,
    }
}

// ── Version detection ─────────────────────────────────────────────────

/// Returns `true` if the raw card JSON is an A2A v0.3 agent
/// (`protocolVersion` starts with `"0."`).
pub fn is_v03(raw: &Value) -> bool {
    raw.get("protocolVersion")
        .and_then(|v| v.as_str())
        .map(|v| v.starts_with("0."))
        .unwrap_or(false)
}

/// Extract the endpoint URL declared in a v0.3 card (`url` field).
/// Falls back to `base_url` when the card does not include a `url` field.
pub fn rpc_url_from_card<'a>(raw: &'a Value, base_url: &'a str) -> &'a str {
    raw.get("url")
        .and_then(|u| u.as_str())
        .unwrap_or(base_url)
}

// ── Key transformation ─────────────────────────────────────────────────

/// Recursively transform every object key in a JSON value using `f`.
fn transform_json_keys(v: Value, f: fn(&str) -> String) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (f(&k), transform_json_keys(v, f)))
                .collect(),
        ),
        Value::Array(arr) => {
            Value::Array(arr.into_iter().map(|v| transform_json_keys(v, f)).collect())
        }
        other => other,
    }
}

/// `camelCase` → `snake_case`.
fn camel_to_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, c) in s.char_indices() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// `snake_case` → `camelCase`.
fn snake_to_camel(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut capitalize_next = false;
    for c in s.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            out.extend(c.to_uppercase());
            capitalize_next = false;
        } else {
            out.push(c);
        }
    }
    out
}

// ── Card normalization ─────────────────────────────────────────────────

/// Up-convert a v0.3 agent card JSON into a v1 [`AgentCard`].
///
/// Handles:
/// - `supportsAuthenticatedExtendedCard: true` → `capabilities.extendedAgentCard: true`
/// - `url` + `preferredTransport` → `supportedInterfaces`
/// - `type: "oauth2"` security schemes → `oauth2SecurityScheme` wrapper
pub fn normalize_card(raw: &Value) -> Result<AgentCard, V03Error> {
    use serde_json::{json, Map};

    let mut v1 = raw.as_object().cloned().unwrap_or_default();

    // v0.3 `supportsAuthenticatedExtendedCard` → v1 `capabilities.extendedAgentCard`
    if v1
        .get("supportsAuthenticatedExtendedCard")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let caps = v1.entry("capabilities").or_insert_with(|| json!({}));
        if let Some(obj) = caps.as_object_mut() {
            obj.entry("extendedAgentCard").or_insert(json!(true));
        }
    }

    // v0.3 has a top-level `url` pointing to the RPC endpoint.
    // Convert to `supportedInterfaces` array.
    if !v1.contains_key("supportedInterfaces")
        && let Some(url) = v1.get("url").and_then(|u| u.as_str())
    {
        let binding = v1
            .get("preferredTransport")
            .and_then(|t| t.as_str())
            .unwrap_or("JSONRPC");
        v1.insert(
            "supportedInterfaces".into(),
            json!([{ "url": url, "protocolBinding": binding, "protocolVersion": "1.0" }]),
        );
    }

    // Normalize security schemes: v0.3 uses `type: "oauth2"`, v1 uses `oauth2SecurityScheme`.
    if let Some(schemes) = v1.get("securitySchemes").and_then(|s| s.as_object()).cloned() {
        let mut normalized: Map<String, Value> = Map::new();
        for (name, scheme) in &schemes {
            if scheme.get("oauth2SecurityScheme").is_some() {
                normalized.insert(name.clone(), scheme.clone()); // already v1
            } else if scheme.get("type").and_then(|t| t.as_str()) == Some("oauth2") {
                let mut inner = scheme.as_object().cloned().unwrap_or_default();
                inner.remove("type");
                normalized.insert(name.clone(), json!({ "oauth2SecurityScheme": inner }));
            } else {
                normalized.insert(name.clone(), scheme.clone());
            }
        }
        v1.insert("securitySchemes".into(), Value::Object(normalized));
    }

    serde_json::from_value(Value::Object(v1))
        .map_err(|e| V03Error::CardParse(e.to_string()))
}

// ── Message parameters ─────────────────────────────────────────────────

/// Parameters for a v0.3 `message/send` or `message/stream` call.
#[derive(Debug, Default)]
pub struct MessageParams {
    pub text: String,
    pub context_id: Option<String>,
    pub task_id: Option<String>,
    pub history_length: Option<i32>,
    pub return_immediately: bool,
    pub tenant: Option<String>,
}

impl MessageParams {
    fn to_json(&self) -> Value {
        serde_json::json!({
            "message": {
                "role": "user",
                "parts": [{ "type": "text", "text": self.text }],
                "messageId": uuid::Uuid::new_v4().to_string(),
                "contextId": self.context_id,
                "taskId": self.task_id,
            },
            "config": {
                "blocking": !self.return_immediately,
                "historyLength": self.history_length,
                "acceptedOutputModes": ["text/plain", "text/markdown"],
            },
            "metadata": { "tenant": self.tenant },
        })
    }

    /// Same as [`to_json`] but with all object keys in `snake_case` for REST transport.
    fn to_snake_json(&self) -> Value {
        transform_json_keys(self.to_json(), camel_to_snake)
    }
}

// ── Client ────────────────────────────────────────────────────────────

/// JSON-RPC or REST client for A2A v0.3 agents.
pub struct Client {
    base_url: String,
    transport: Transport,
    http: reqwest::Client,
}

impl Client {
    /// Construct a new client for the given `base_url`.
    ///
    /// - `transport` selects the wire protocol: use [`transport_from_card`] to
    ///   detect the correct value from the agent card.
    /// - `bearer` is attached as `Authorization: Bearer <token>` on every request.
    pub fn new(base_url: &str, bearer: Option<&str>, transport: Transport) -> Result<Self, V03Error> {
        let http = build_http_client(bearer)?;
        Ok(Self { base_url: base_url.to_string(), transport, http })
    }

    // ── JSON-RPC helpers ──────────────────────────────────────────────

    /// Execute a JSON-RPC method and return the `result` field of the response.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, V03Error> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": method,
            "params": params,
        });
        let json: Value = self
            .http
            .post(&self.base_url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if let Some(err) = json.get("error") {
            let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
            return Err(V03Error::Rpc(format!("jsonrpc error: {msg}")));
        }
        Ok(json.get("result").cloned().unwrap_or(json))
    }

    // ── REST helpers ──────────────────────────────────────────────────

    /// Send a REST request and return the response as camelCase JSON.
    async fn rest_request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        query: &[(&str, &str)],
    ) -> Result<Value, V03Error> {
        let url = format!("{}{}", self.base_url.trim_end_matches('/'), path);
        let mut req = match method {
            "GET" => self.http.get(&url),
            "POST" => self.http.post(&url),
            "DELETE" => self.http.delete(&url),
            m => return Err(V03Error::Rpc(format!("unsupported HTTP method: {m}"))),
        };
        if !query.is_empty() {
            req = req.query(query);
        }
        if let Some(b) = body {
            req = req.header("Content-Type", "application/json").json(&b);
        }
        let resp = req.header("Accept", "application/json").send().await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(V03Error::Rpc(format!("HTTP {status}: {text}")));
        }
        // REST responses are snake_case; normalise to camelCase for consistency.
        let snake: Value = resp.json().await?;
        Ok(transform_json_keys(snake, snake_to_camel))
    }

    // ── Public API ────────────────────────────────────────────────────

    /// Send a message and return the response value.
    pub async fn send_message(&self, params: &MessageParams) -> Result<Value, V03Error> {
        match self.transport {
            Transport::JsonRpc => self.call("message/send", params.to_json()).await,
            Transport::Rest => {
                self.rest_request("POST", "/message:send", Some(params.to_snake_json()), &[])
                    .await
            }
        }
    }

    /// Fetch a task by ID.
    pub async fn get_task(&self, id: &str, history_length: Option<i32>) -> Result<Value, V03Error> {
        match self.transport {
            Transport::JsonRpc => {
                self.call(
                    "tasks/get",
                    serde_json::json!({ "id": id, "historyLength": history_length }),
                )
                .await
            }
            Transport::Rest => {
                let hl_str = history_length.map(|hl| hl.to_string());
                let mut query: Vec<(&str, &str)> = vec![];
                if let Some(ref hl) = hl_str {
                    query.push(("historyLength", hl.as_str()));
                }
                self.rest_request("GET", &format!("/tasks/{id}"), None, &query).await
            }
        }
    }

    /// List tasks with optional filters.
    ///
    /// Returns [`V03Error::Unsupported`] over JSON-RPC (v0.3 JSON-RPC does not
    /// define a `tasks/list` method). Works over REST (`GET /tasks`).
    pub async fn list_tasks(
        &self,
        context_id: Option<&str>,
        page_size: Option<i32>,
        page_token: Option<&str>,
    ) -> Result<Value, V03Error> {
        match self.transport {
            Transport::JsonRpc => Err(V03Error::Unsupported(
                "list-tasks is not supported over JSON-RPC for v0.3 agents",
            )),
            Transport::Rest => {
                let ps_str = page_size.map(|ps| ps.to_string());
                let mut query: Vec<(&str, &str)> = vec![];
                if let Some(ctx) = context_id {
                    query.push(("contextId", ctx));
                }
                if let Some(ref ps) = ps_str {
                    query.push(("pageSize", ps.as_str()));
                }
                if let Some(pt) = page_token {
                    query.push(("pageToken", pt));
                }
                self.rest_request("GET", "/tasks", None, &query).await
            }
        }
    }

    /// Cancel a running task.
    pub async fn cancel_task(&self, id: &str) -> Result<Value, V03Error> {
        match self.transport {
            Transport::JsonRpc => {
                self.call("tasks/cancel", serde_json::json!({ "id": id })).await
            }
            Transport::Rest => {
                self.rest_request("POST", &format!("/tasks/{id}:cancel"), None, &[]).await
            }
        }
    }

    /// Stream a message, calling `on_event` for each SSE event received.
    pub async fn stream_message<E>(
        &self,
        params: &MessageParams,
        on_event: impl FnMut(Value) -> Result<(), E>,
    ) -> Result<(), SseError<E>> {
        match self.transport {
            Transport::JsonRpc => {
                self.stream_jsonrpc_sse("message/stream", params.to_json(), on_event).await
            }
            Transport::Rest => {
                self.stream_rest_sse("/message:stream", Some(params.to_snake_json()), on_event)
                    .await
            }
        }
    }

    /// Subscribe to live task updates, calling `on_event` for each SSE event.
    pub async fn subscribe<E>(
        &self,
        id: &str,
        on_event: impl FnMut(Value) -> Result<(), E>,
    ) -> Result<(), SseError<E>> {
        match self.transport {
            Transport::JsonRpc => {
                self.stream_jsonrpc_sse(
                    "tasks/resubscribe",
                    serde_json::json!({ "id": id }),
                    on_event,
                )
                .await
            }
            Transport::Rest => {
                self.stream_rest_sse(&format!("/tasks/{id}:subscribe"), None, on_event).await
            }
        }
    }

    // ── Private streaming helpers ─────────────────────────────────────

    /// Stream over JSON-RPC SSE, unwrapping the JSON-RPC envelope on each event.
    async fn stream_jsonrpc_sse<E>(
        &self,
        method: &str,
        params: Value,
        mut on_event: impl FnMut(Value) -> Result<(), E>,
    ) -> Result<(), SseError<E>> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": method,
            "params": params,
        });
        let resp = self
            .http
            .post(&self.base_url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| SseError::Protocol(V03Error::Http(e)))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(SseError::Protocol(V03Error::Rpc(format!("HTTP {status}: {text}"))));
        }

        // Unwrap JSON-RPC envelope: { "result": <event> } or { "error": {...} }
        parse_sse_stream(resp, |frame| {
            if let Some(result) = frame.get("result").cloned() {
                on_event(result).map_err(SseError::Callback)
            } else if let Some(err) = frame.get("error") {
                let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
                Err(SseError::Protocol(V03Error::Rpc(format!("jsonrpc error: {msg}"))))
            } else {
                Ok(()) // keep-alive or unrecognised frame — skip
            }
        })
        .await
    }

    /// Stream over REST SSE, transforming snake_case event keys to camelCase.
    async fn stream_rest_sse<E>(
        &self,
        path: &str,
        body: Option<Value>,
        mut on_event: impl FnMut(Value) -> Result<(), E>,
    ) -> Result<(), SseError<E>> {
        let url = format!("{}{}", self.base_url.trim_end_matches('/'), path);
        let mut req = self.http.post(&url).header("Accept", "text/event-stream");
        if let Some(b) = body {
            req = req.header("Content-Type", "application/json").json(&b);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| SseError::Protocol(V03Error::Http(e)))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(SseError::Protocol(V03Error::Rpc(format!("HTTP {status}: {text}"))));
        }

        // REST events are snake_case; normalise to camelCase before forwarding.
        parse_sse_stream(resp, |event| {
            on_event(transform_json_keys(event, snake_to_camel)).map_err(SseError::Callback)
        })
        .await
    }
}

// ── SSE stream parser ─────────────────────────────────────────────────

/// Parse `data:` lines from an SSE response, calling `on_line` for each
/// non-empty, non-`[DONE]` JSON line.
async fn parse_sse_stream<E>(
    resp: reqwest::Response,
    mut on_line: impl FnMut(Value) -> Result<(), SseError<E>>,
) -> Result<(), SseError<E>> {
    let byte_stream = resp.bytes_stream().map(|r| r.map_err(std::io::Error::other));
    let mut reader = tokio::io::BufReader::new(StreamReader::new(byte_stream));
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| SseError::Protocol(V03Error::Io(e)))?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim_start_matches("data:").trim();
        if trimmed.is_empty() || trimmed == "[DONE]" {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
            on_line(v)?;
        }
    }
    Ok(())
}

// ── HTTP client helper ────────────────────────────────────────────────

fn build_http_client(bearer: Option<&str>) -> Result<reqwest::Client, V03Error> {
    let mut builder = reqwest::Client::builder();
    if let Some(token) = bearer {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {token}")
                .parse()
                .map_err(|e| V03Error::Rpc(format!("invalid bearer token: {e}")))?,
        );
        builder = builder.default_headers(headers);
    }
    builder.build().map_err(V03Error::Http)
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_v03 ────────────────────────────────────────────────────────

    #[test]
    fn is_v03_with_zero_dot_prefix() {
        assert!(is_v03(&serde_json::json!({"protocolVersion": "0.3.0"})));
    }

    #[test]
    fn is_v03_returns_false_for_v1() {
        assert!(!is_v03(&serde_json::json!({"protocolVersion": "1.0"})));
    }

    #[test]
    fn is_v03_returns_false_when_key_missing() {
        assert!(!is_v03(&serde_json::json!({"name": "Agent"})));
    }

    #[test]
    fn is_v03_returns_false_for_non_string_version() {
        assert!(!is_v03(&serde_json::json!({"protocolVersion": 3})));
    }

    // ── transport_from_card ───────────────────────────────────────────

    #[test]
    fn transport_from_card_http_json_returns_rest() {
        let raw = serde_json::json!({"preferredTransport": "HTTP+JSON"});
        assert_eq!(transport_from_card(&raw), Transport::Rest);
    }

    #[test]
    fn transport_from_card_jsonrpc_returns_jsonrpc() {
        let raw = serde_json::json!({"preferredTransport": "JSONRPC"});
        assert_eq!(transport_from_card(&raw), Transport::JsonRpc);
    }

    #[test]
    fn transport_from_card_missing_defaults_to_jsonrpc() {
        assert_eq!(transport_from_card(&serde_json::json!({})), Transport::JsonRpc);
    }

    // ── rpc_url_from_card ─────────────────────────────────────────────

    #[test]
    fn rpc_url_from_card_uses_url_field() {
        let raw = serde_json::json!({"url": "https://example.com/rpc"});
        assert_eq!(rpc_url_from_card(&raw, "https://fallback.example.com"), "https://example.com/rpc");
    }

    #[test]
    fn rpc_url_from_card_falls_back_to_base() {
        let raw = serde_json::json!({"name": "Agent"});
        assert_eq!(rpc_url_from_card(&raw, "https://fallback.example.com"), "https://fallback.example.com");
    }

    // ── camel_to_snake / snake_to_camel ───────────────────────────────

    #[test]
    fn camel_to_snake_converts() {
        assert_eq!(camel_to_snake("messageId"), "message_id");
        assert_eq!(camel_to_snake("contextId"), "context_id");
        assert_eq!(camel_to_snake("historyLength"), "history_length");
        assert_eq!(camel_to_snake("acceptedOutputModes"), "accepted_output_modes");
        assert_eq!(camel_to_snake("role"), "role"); // no change
    }

    #[test]
    fn snake_to_camel_converts() {
        assert_eq!(snake_to_camel("message_id"), "messageId");
        assert_eq!(snake_to_camel("context_id"), "contextId");
        assert_eq!(snake_to_camel("history_length"), "historyLength");
        assert_eq!(snake_to_camel("next_page_token"), "nextPageToken");
        assert_eq!(snake_to_camel("role"), "role"); // no change
    }

    #[test]
    fn transform_json_keys_nested() {
        let v = serde_json::json!({
            "contextId": "ctx",
            "status": { "taskState": "completed" },
            "parts": [{ "textPart": "hello" }]
        });
        let result = transform_json_keys(v, camel_to_snake);
        assert_eq!(result["context_id"], "ctx");
        assert_eq!(result["status"]["task_state"], "completed");
        assert_eq!(result["parts"][0]["text_part"], "hello");
    }

    // ── normalize_card ─────────────────────────────────────────────────

    fn minimal_v03_json(url: &str) -> serde_json::Value {
        serde_json::json!({
            "name": "Test Agent",
            "description": "A test",
            "version": "1.0.0",
            "protocolVersion": "0.3.0",
            "url": url,
            "preferredTransport": "JSONRPC",
            "capabilities": {},
            "defaultInputModes": ["text/plain"],
            "defaultOutputModes": ["text/plain"],
            "skills": []
        })
    }

    #[test]
    fn normalize_adds_supported_interfaces_from_url() {
        let raw = minimal_v03_json("https://example.com/rpc");
        let card = normalize_card(&raw).unwrap();
        assert!(!card.supported_interfaces.is_empty());
        assert_eq!(card.supported_interfaces[0].url, "https://example.com/rpc");
    }

    #[test]
    fn normalize_normalizes_security_scheme() {
        let mut raw = minimal_v03_json("https://example.com/rpc");
        raw["securitySchemes"] = serde_json::json!({
            "myOAuth": {
                "type": "oauth2",
                "flows": {
                    "authorizationCode": {
                        "authorizationUrl": "https://a.example.com/auth",
                        "tokenUrl": "https://a.example.com/token",
                        "scopes": {}
                    }
                }
            }
        });
        let card = normalize_card(&raw).unwrap();
        assert!(card.security_schemes.is_some());
    }

    #[test]
    fn normalize_keeps_existing_supported_interfaces() {
        let mut raw = minimal_v03_json("https://example.com/rpc");
        raw["supportedInterfaces"] = serde_json::json!([{
            "url": "https://other.example.com/rpc",
            "protocolBinding": "JSONRPC",
            "protocolVersion": "1.0"
        }]);
        let card = normalize_card(&raw).unwrap();
        assert_eq!(card.supported_interfaces[0].url, "https://other.example.com/rpc");
    }

    #[test]
    fn normalize_maps_supports_authenticated_extended_card() {
        let mut raw = minimal_v03_json("https://example.com/rpc");
        raw["supportsAuthenticatedExtendedCard"] = serde_json::json!(true);
        let card = normalize_card(&raw).unwrap();
        assert!(card.capabilities.extended_agent_card);
    }

    #[test]
    fn normalize_does_not_override_existing_extended_card_false() {
        // If capabilities.extendedAgentCard is already set to true, it stays true.
        let mut raw = minimal_v03_json("https://example.com/rpc");
        raw["supportsAuthenticatedExtendedCard"] = serde_json::json!(true);
        raw["capabilities"] = serde_json::json!({ "extendedAgentCard": true });
        let card = normalize_card(&raw).unwrap();
        assert!(card.capabilities.extended_agent_card);
    }
}
