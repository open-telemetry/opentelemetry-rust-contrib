//! Mapping from HTTP status codes to OpenTelemetry span [`Status`].

use opentelemetry::trace::Status;

/// Span status for a server span.
///
/// Per the HTTP semantic conventions, a server span is only marked as an error
/// for `5xx` responses; `4xx` responses are considered the caller's fault and
/// leave the span status unset.
#[cfg(feature = "http-server")]
pub(crate) fn server_status(status: http::StatusCode) -> Option<Status> {
    status.is_server_error().then(|| Status::Error {
        description: format!("HTTP {}", status.as_u16()).into(),
    })
}

/// Span status for a client span.
///
/// Per the HTTP semantic conventions, a client span is marked as an error for
/// both `4xx` and `5xx` responses.
#[cfg(feature = "http-client")]
pub(crate) fn client_status(status: http::StatusCode) -> Option<Status> {
    (status.is_client_error() || status.is_server_error()).then(|| Status::Error {
        description: format!("HTTP {}", status.as_u16()).into(),
    })
}
