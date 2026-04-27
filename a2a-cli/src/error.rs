use thiserror::Error;

#[derive(Debug, Error)]
pub enum A2aCliError {
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
    /// before falling back to a full re-authentication (`a2a auth login`).
    #[error("auth error: token expired — run `a2a auth login` to re-authenticate")]
    AuthExpired,

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("v0.3 error: {0}")]
    V03(#[from] a2a_compat::V03Error),
}

impl A2aCliError {
    /// Exit code for this error (mirrors gws-cli convention).
    pub fn exit_code(&self) -> i32 {
        match self {
            A2aCliError::A2A(_) | A2aCliError::Http(_) | A2aCliError::V03(_) => 1,
            A2aCliError::Auth(_) | A2aCliError::AuthExpired => 2,
            A2aCliError::InvalidInput(_) => 3,
            A2aCliError::Config(_) => 4,
            _ => 5,
        }
    }

    /// Wrap this error with additional context about what was happening.
    /// The context is prepended to the error message: "{context}: {self}".
    pub fn context(self, ctx: impl std::fmt::Display) -> Self {
        match self {
            A2aCliError::A2A(e) => A2aCliError::A2A(a2a::A2AError::internal(format!("{ctx}: {e}"))),
            A2aCliError::Auth(msg) => A2aCliError::Auth(format!("{ctx}: {msg}")),
            A2aCliError::Config(msg) => A2aCliError::Config(format!("{ctx}: {msg}")),
            A2aCliError::InvalidInput(msg) => A2aCliError::InvalidInput(format!("{ctx}: {msg}")),
            // NOTE: Http, V03, Io, Json variants don't carry a mutable String payload,
            // so we convert to InvalidInput (exit code 3) to attach context.
            // This changes the exit code from the variant's natural code (1 or 5).
            // If you need to preserve the exit code, add an explicit arm above.
            other => A2aCliError::InvalidInput(format!("{ctx}: {other}")),
        }
    }

    /// Whether retrying the same request might succeed.
    ///
    /// HTTP 5xx / connection errors are transient; 4xx, auth, config, and
    /// invalid-input errors are permanent — retrying won't help.
    /// For A2A errors, only INTERNAL_ERROR (-32603) is transient; domain errors
    /// like TASK_NOT_FOUND or INVALID_PARAMS are permanent.
    pub fn is_retryable(&self) -> bool {
        match self {
            A2aCliError::Http(e) => {
                // Connection-level errors (timeout, reset) are retryable.
                // HTTP 5xx responses are retryable; 4xx are permanent.
                e.is_timeout() || e.is_connect() || e.status().is_some_and(|s| s.is_server_error())
            }
            // Only server-side internal errors are transient; all domain/client errors are permanent.
            A2aCliError::A2A(e) => e.code == a2a::error_code::INTERNAL_ERROR,
            // V03: HTTP transport errors inherit the same rule; RPC/IO/parse errors are permanent.
            A2aCliError::V03(a2a_compat::V03Error::Http(e)) => {
                e.is_timeout() || e.is_connect() || e.status().is_some_and(|s| s.is_server_error())
            }
            A2aCliError::V03(_) => false,
            A2aCliError::Auth(_)
            | A2aCliError::AuthExpired
            | A2aCliError::Config(_)
            | A2aCliError::InvalidInput(_)
            | A2aCliError::Json(_)
            | A2aCliError::Io(_) => false,
        }
    }
}

/// Converts a streaming SSE error into `A2aCliError`.
///
/// `SseError::Protocol(e)` maps to `A2aCliError::V03(e)`.
/// `SseError::Callback(e)` unwraps the inner `A2aCliError` (already the right type).
impl From<a2a_compat::SseError<A2aCliError>> for A2aCliError {
    fn from(e: a2a_compat::SseError<A2aCliError>) -> Self {
        match e {
            a2a_compat::SseError::Protocol(e) => A2aCliError::V03(e),
            a2a_compat::SseError::Callback(e) => e,
        }
    }
}

pub type Result<T> = std::result::Result<T, A2aCliError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_a2a_error() {
        assert_eq!(
            A2aCliError::A2A(a2a::A2AError::internal("x")).exit_code(),
            1
        );
    }

    #[test]
    fn exit_code_auth_error() {
        assert_eq!(A2aCliError::Auth("x".to_string()).exit_code(), 2);
    }

    #[test]
    fn exit_code_invalid_input() {
        assert_eq!(A2aCliError::InvalidInput("x".to_string()).exit_code(), 3);
    }

    #[test]
    fn exit_code_config_error() {
        assert_eq!(A2aCliError::Config("x".to_string()).exit_code(), 4);
    }

    #[test]
    fn exit_code_io_error() {
        let e = A2aCliError::Io(std::io::Error::new(std::io::ErrorKind::Other, "x"));
        assert_eq!(e.exit_code(), 5);
    }

    #[test]
    fn is_retryable_auth_is_false() {
        assert!(!A2aCliError::Auth("expired".into()).is_retryable());
    }

    #[test]
    fn is_retryable_config_is_false() {
        assert!(!A2aCliError::Config("bad config".into()).is_retryable());
    }

    #[test]
    fn is_retryable_invalid_input_is_false() {
        assert!(!A2aCliError::InvalidInput("bad arg".into()).is_retryable());
    }

    #[test]
    fn is_retryable_a2a_internal_error_is_true() {
        assert!(A2aCliError::A2A(a2a::A2AError::internal("server error")).is_retryable());
    }

    #[test]
    fn is_retryable_a2a_domain_error_is_false() {
        assert!(!A2aCliError::A2A(a2a::A2AError::task_not_found("t1")).is_retryable());
        assert!(!A2aCliError::A2A(a2a::A2AError::invalid_params("bad")).is_retryable());
        assert!(!A2aCliError::A2A(a2a::A2AError::task_not_cancelable("t1")).is_retryable());
    }

    #[test]
    fn is_retryable_v03_rpc_error_is_false() {
        assert!(
            !A2aCliError::V03(a2a_compat::V03Error::Rpc("method not found".into())).is_retryable()
        );
    }

    #[test]
    fn exit_code_json_error() {
        let e: A2aCliError = serde_json::from_str::<serde_json::Value>("bad")
            .unwrap_err()
            .into();
        assert_eq!(e.exit_code(), 5);
    }
}
