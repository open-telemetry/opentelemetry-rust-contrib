//! [OpenTelemetry] instrumentation middleware for [Tower]-compatible HTTP servers
//! (Axum, Hyper, Tonic, etc.).
//!
//! The middleware produces both metrics and distributed tracing for incoming HTTP
//! requests, following the OpenTelemetry [HTTP semantic conventions].
//!
//! # Layout
//!
//! - [`http::server`] — instruments incoming requests (`SpanKind::Server`) and
//!   extracts the trace context from request headers.
//! - [`http::extractors`] — pluggable route and attribute extractors used by the
//!   server layer.
//!
//! # Metrics
//!
//! - `http.server.request.duration` — duration of HTTP server requests.
//! - `http.server.active_requests` — number of in-flight HTTP server requests.
//! - `http.server.request.body.size` — size of HTTP server request bodies.
//! - `http.server.response.body.size` — size of HTTP server response bodies.
//!
//! # Tracing
//!
//! A server span (`SpanKind::Server`) is created per request, with attributes such
//! as `http.request.method`, `url.scheme`, `url.path`, `url.full`,
//! `user_agent.original`, `http.route`, and `http.response.status_code`.
//!
//! # Quick start
//!
//! With the default `axum` feature enabled, applying the middleware is a single
//! [`http::server::Layer::new`] call:
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
//! The layer reads the global [`TracerProvider`] and [`MeterProvider`], so configure
//! those before constructing the layer.
//!
//! # Customization
//!
//! Use [`http::server::LayerBuilder`] to plug in custom extractors:
//!
//! - [`http::extractors::RouteExtractor`] decides how the `http.route` attribute
//!   (and span name) is produced. Built-in choices:
//!   [`http::extractors::NoRouteExtractor`], [`http::extractors::PathExtractor`],
//!   [`http::extractors::AxumMatchedPathExtractor`] (requires the `axum` feature),
//!   or [`http::extractors::FnRouteExtractor`].
//! - [`http::extractors::RequestAttributeExtractor`] /
//!   [`http::extractors::ResponseAttributeExtractor`] let you attach additional
//!   attributes to spans and metrics. The default is
//!   [`http::extractors::NoOpExtractor`].
//!
//! See [`http::extractors::RouteExtractor`] for cardinality guidance — picking the
//! wrong extractor can blow up the cardinality of your metrics.
//!
//! # Cargo features
//!
//! - `axum` *(default-off)* — enables
//!   [`http::extractors::AxumMatchedPathExtractor`] and makes it the default route
//!   extractor. Without this feature the default extractor is
//!   [`http::extractors::NoRouteExtractor`] (method-only span names, no
//!   `http.route` attribute).
//!
//! # Examples
//!
//! Runnable end-to-end examples live in the [`examples/`] directory of the
//! `opentelemetry-rust-contrib` repository.
//!
//! [OpenTelemetry]: https://opentelemetry.io
//! [Tower]: https://docs.rs/tower
//! [HTTP semantic conventions]: https://opentelemetry.io/docs/specs/semconv/http/
//! [`TracerProvider`]: opentelemetry::trace::TracerProvider
//! [`MeterProvider`]: opentelemetry::metrics::MeterProvider
//! [`examples/`]: https://github.com/open-telemetry/opentelemetry-rust-contrib/tree/main/opentelemetry-instrumentation-tower/examples

use std::fmt;

mod common;
pub mod http;

/// Instrumentation scope name reported on emitted spans and metrics.
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
