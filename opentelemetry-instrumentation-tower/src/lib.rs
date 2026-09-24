//! [OpenTelemetry] instrumentation middleware for [Tower]-compatible HTTP clients
//! and servers (Axum, Hyper, reqwest, etc.).
//!
//! The middleware produces both metrics and distributed tracing following the
//! OpenTelemetry [HTTP semantic conventions].
//!
//! # Layout
//!
//! - [`http::server`] — instruments incoming requests (`SpanKind::Server`) and
//!   extracts the trace context from request headers.
//! - [`http::client`] — instruments outgoing requests (`SpanKind::Client`) and
//!   injects the trace context into request headers.
//! - [`http::extractors`] — pluggable route and attribute extractors shared by
//!   the server and client layers.
//!
//! # Metrics
//!
//! - `http.server.request.duration` — duration of HTTP server requests.
//! - `http.server.active_requests` — number of in-flight HTTP server requests.
//! - `http.server.request.body.size` — size of HTTP server request bodies.
//! - `http.server.response.body.size` — size of HTTP server response bodies.
//! - `http.client.request.duration` — duration of HTTP client requests.
//! - `http.client.request.body.size` — size of HTTP client request bodies.
//! - `http.client.response.body.size` — size of HTTP client response bodies.
//!
//! # Tracing
//!
//! A server span (`SpanKind::Server`) is created per request, with attributes such
//! as `http.request.method`, `url.scheme`, `url.path`, `url.query`,
//! `user_agent.original`, `http.route`, and `http.response.status_code`.
//!
//! A client span (`SpanKind::Client`) is created per request, with attributes such
//! as `http.request.method`, `url.full`, `url.scheme`, `server.address`,
//! `server.port`, and `http.response.status_code`.
//!
//! The value of a query parameter that can carry a credential, such as `sig`, is
//! replaced with `REDACTED` in `url.query` and `url.full`. The client layer also
//! replaces the user information of `url.full` with `REDACTED:REDACTED`. Use
//! [`http::server::LayerBuilder::with_sensitive_query_parameters`] and
//! [`http::client::LayerBuilder::with_sensitive_query_parameters`] to name the
//! keys yourself.
//!
//! # Quick start
//!
//! ```ignore
//! use axum::{routing::get, Router};
//! use opentelemetry_instrumentation_tower::http;
//!
//! # async fn root() -> &'static str { "hello" }
//! # fn run() {
//! let app: Router = Router::new()
//!     .route("/", get(root))
//!     // Apply the OTel layer *after* the routes so that
//!     // `AxumMatchedPathExtractor` can read the matched route template.
//!     .layer(http::server::Layer::new());
//! # let _ = app;
//! # }
//! ```
//!
//! Instrument an outgoing client stack:
//!
//! ```ignore
//! use opentelemetry_instrumentation_tower::http;
//! use tower::ServiceBuilder;
//!
//! # fn run<S>(inner: S) {
//! let client = ServiceBuilder::new()
//!     // Tracing and metrics are on by default; toggle either per layer.
//!     .layer(
//!         http::client::LayerBuilder::builder()
//!             .with_metrics(false)
//!             .build()
//!             .unwrap(),
//!     )
//!     .service(inner);
//! # let _ = client;
//! # }
//! ```
//!
//! The layers read the global [`TracerProvider`] and [`MeterProvider`], so
//! configure those (and a text-map propagator) before constructing them.
//!
//! # Cargo features
//!
//! - `http-server` *(default)* — the HTTP server layer ([`http::server`]).
//! - `http-client` *(default)* — the HTTP client layer ([`http::client`]).
//! - `axum` — enables [`http::extractors::AxumMatchedPathExtractor`] and makes it
//!   the default route extractor for the server layer.
//!
//! [OpenTelemetry]: https://opentelemetry.io
//! [Tower]: https://docs.rs/tower
//! [HTTP semantic conventions]: https://opentelemetry.io/docs/specs/semconv/http/
//! [`TracerProvider`]: opentelemetry::trace::TracerProvider
//! [`MeterProvider`]: opentelemetry::metrics::MeterProvider

use std::fmt;

#[cfg(any(feature = "http-server", feature = "http-client"))]
mod common;
#[cfg(any(feature = "http-server", feature = "http-client"))]
pub mod http;

/// Instrumentation scope name reported on emitted spans and metrics.
#[cfg(any(feature = "http-server", feature = "http-client"))]
pub(crate) const INSTRUMENTATION_NAME: &str = "opentelemetry-instrumentation-tower";

/// Error type for `opentelemetry_instrumentation_tower`.
pub struct Error {
    #[allow(dead_code)]
    inner: ErrorKind,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.inner {
            ErrorKind::Other(ref s) => write!(f, "{s}"),
            ErrorKind::Config(ref s) => write!(f, "config error: {s}"),
        }
    }
}

impl std::error::Error for Error {}

/// `Result` typedef to use with the [`Error`] type.
pub type Result<T> = std::result::Result<T, Error>;

enum ErrorKind {
    #[allow(dead_code)]
    /// Uncategorized
    Other(String),
    #[allow(dead_code)]
    /// Invalid configuration
    Config(String),
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_tuple("opentelemetry_instrumentation_tower::Error")
            .finish()
    }
}
