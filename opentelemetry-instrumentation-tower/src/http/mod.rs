//! HTTP instrumentation: server and client [`tower`] layers.
//!
//! - [`server`] instruments incoming requests (`SpanKind::Server`).
//! - [`client`] instruments outgoing requests (`SpanKind::Client`).
//! - [`extractors`] holds the pluggable route and attribute extractors shared by
//!   both.
//!
//! [`tower`]: https://docs.rs/tower

pub mod extractors;

#[cfg(feature = "http-server")]
pub mod server;

#[cfg(feature = "http-client")]
pub mod client;
