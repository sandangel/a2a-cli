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

    #[error("auth error: {0}")]
    Auth(String),

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
            AgcError::Auth(_) => 2,
            AgcError::InvalidInput(_) => 3,
            AgcError::Config(_) => 4,
            _ => 5,
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
    fn exit_code_json_error() {
        let e: AgcError = serde_json::from_str::<serde_json::Value>("bad")
            .unwrap_err()
            .into();
        assert_eq!(e.exit_code(), 5);
    }
}
