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
}

impl AgcError {
    /// Exit code for this error (mirrors gws-cli convention).
    pub fn exit_code(&self) -> i32 {
        match self {
            AgcError::A2A(_) | AgcError::Http(_) => 1,
            AgcError::Auth(_) => 2,
            AgcError::InvalidInput(_) => 3,
            AgcError::Config(_) => 4,
            _ => 5,
        }
    }
}

pub type Result<T> = std::result::Result<T, AgcError>;
