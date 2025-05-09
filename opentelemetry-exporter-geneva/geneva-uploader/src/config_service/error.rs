use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum GenevaConfigClientError {
    // Authentication-related errors
    #[error("Authentication method not implemented: {0}")]
    AuthMethodNotImplemented(String),
    #[error("Missing Auth Info: {0}")]
    AuthInfoNotFound(String),
    #[error("Invalid or malformed JWT token: {0}")]
    JwtTokenError(String),
    #[error("Certificate error: {0}")]
    Certificate(String),

    // Networking / HTTP / TLS
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Request failed with status {status}: {message}")]
    RequestFailed { status: u16, message: String },

    // Data / parsing
    #[error("JSON error: {0}")]
    SerdeJson(#[from] serde_json::Error),

    // Misc
    #[error("Moniker not found: {0}")]
    MonikerNotFound(String),
    #[error("Internal error: {0}")]
    InternalError(String),
}

#[allow(dead_code)]
pub(crate) type GenevaConfigClientResult<T> = std::result::Result<T, GenevaConfigClientError>;
