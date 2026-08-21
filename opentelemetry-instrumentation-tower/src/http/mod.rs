//! HTTP instrumentation: the server [`tower`] layer and its extractors.
//!
//! - [`server`] instruments incoming requests (`SpanKind::Server`).
//! - [`extractors`] holds the pluggable route and attribute extractors.
//!
//! [`tower`]: https://docs.rs/tower

pub mod extractors;
pub mod server;
