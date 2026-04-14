use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgcError {
    #[error("a2a error: {0}")]
    A2A(#[from] a2a::A2AError),

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("config error: {0}")]
    Config(String),

    /// Authentication failed — credentials invalid or missing.
    #[error("auth error: {0}")]
    Auth(String),

    /// Access token is expired. Callers can attempt a silent token refresh
    /// before falling back to a full re-authentication (`agc auth login`).
    #[error("auth error: token expired — run `agc auth login` to re-authenticate")]
    AuthExpired,

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("v0.3 error: {0}")]
    V03(#[from] a2a_compat::V03Error),
}

impl AgcError {
    /// Exit code for this error (mirrors gws-cli convention).
    pub fn exit_code(&self) -> i32 {
        match self {
            AgcError::A2A(_) | AgcError::Http(_) | AgcError::V03(_) => 1,
            AgcError::Auth(_) | AgcError::AuthExpired => 2,
            AgcError::InvalidInput(_) => 3,
            AgcError::Config(_) => 4,
            _ => 5,
        }
    }

    /// Whether retrying the same request might succeed.
    ///
    /// HTTP 5xx / connection errors are transient; 4xx, auth, config, and
    /// invalid-input errors are permanent — retrying won't help.
    pub fn is_retryable(&self) -> bool {
        match self {
            AgcError::Http(e) => {
                // Connection-level errors (timeout, reset) are retryable.
                // HTTP 5xx responses are retryable; 4xx are permanent.
                e.is_timeout() || e.is_connect() || e.status().is_some_and(|s| s.is_server_error())
            }
            AgcError::A2A(_) | AgcError::V03(_) => true,
            AgcError::Auth(_)
            | AgcError::AuthExpired
            | AgcError::Config(_)
            | AgcError::InvalidInput(_)
            | AgcError::Json(_)
            | AgcError::Io(_) => false,
        }
    }
}

/// Converts a streaming SSE error into `AgcError`.
///
/// `SseError::Protocol(e)` maps to `AgcError::V03(e)`.
/// `SseError::Callback(e)` unwraps the inner `AgcError` (already the right type).
impl From<a2a_compat::SseError<AgcError>> for AgcError {
    fn from(e: a2a_compat::SseError<AgcError>) -> Self {
        match e {
            a2a_compat::SseError::Protocol(e) => AgcError::V03(e),
            a2a_compat::SseError::Callback(e) => e,
        }
    }
}

pub type Result<T> = std::result::Result<T, AgcError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_a2a_error() {
        assert_eq!(AgcError::A2A(a2a::A2AError::internal("x")).exit_code(), 1);
    }

    #[test]
    fn exit_code_auth_error() {
        assert_eq!(AgcError::Auth("x".to_string()).exit_code(), 2);
    }

    #[test]
    fn exit_code_invalid_input() {
        assert_eq!(AgcError::InvalidInput("x".to_string()).exit_code(), 3);
    }

    #[test]
    fn exit_code_config_error() {
        assert_eq!(AgcError::Config("x".to_string()).exit_code(), 4);
    }

    #[test]
    fn exit_code_io_error() {
        let e = AgcError::Io(std::io::Error::new(std::io::ErrorKind::Other, "x"));
        assert_eq!(e.exit_code(), 5);
    }

    #[test]
    fn is_retryable_auth_is_false() {
        assert!(!AgcError::Auth("expired".into()).is_retryable());
    }

    #[test]
    fn is_retryable_config_is_false() {
        assert!(!AgcError::Config("bad config".into()).is_retryable());
    }

    #[test]
    fn is_retryable_invalid_input_is_false() {
        assert!(!AgcError::InvalidInput("bad arg".into()).is_retryable());
    }

    #[test]
    fn is_retryable_a2a_is_true() {
        assert!(AgcError::A2A(a2a::A2AError::internal("server error")).is_retryable());
    }

    #[test]
    fn exit_code_json_error() {
        let e: AgcError = serde_json::from_str::<serde_json::Value>("bad")
            .unwrap_err()
            .into();
        assert_eq!(e.exit_code(), 5);
    }
}
