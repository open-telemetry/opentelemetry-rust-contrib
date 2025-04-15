//! [OpenTelemetry] instrumentation for [Actix Web].
//!
//! This crate allows you to easily instrument client and server requests.
//!
//! * Server requests can be traced by using the [`RequestTracing`] middleware.
//!
//! The `awc` feature allows you to instrument client requests made by the [awc] crate.
//!
//! * Client requests can be traced by using the [`ClientExt::trace_request`] method.
//!
//! The `metrics` feature allows you to export request metrics to any OTLP supported
//! backend like [Prometheus].
//!
//! * Metrics can be tracked using the [`RequestMetrics`] middleware.
//!
//! [OpenTelemetry]: https://opentelemetry.io
//! [Actix Web]: https://actix.rs
//! [awc]: https://docs.rs/awc
//! [Prometheus]: https://prometheus.io
//!
//! ### Client Request Examples:
//!
//! Note: this requires the `awc` feature to be enabled.
//!
//! ```no_run
//! # #[cfg(feature="awc")]
//! # {
//! use awc::{Client, error::SendRequestError};
//! use opentelemetry_instrumentation_actix_web::ClientExt;
//!
//! async fn execute_request(client: &Client) -> Result<(), SendRequestError> {
//!     let res = client
//!         .get("http://localhost:8080")
//!         // Add `trace_request` before `send` to any awc request to add instrumentation
//!         .trace_request()
//!         .send()
//!         .await?;
//!
//!     println!("Response: {:?}", res);
//!     Ok(())
//! }
//! # }
//! ```
//!
//! ### Server middleware examples:
//!
//! Tracing and metrics middleware can be used together or independently.
//!
//! Tracing server example:
//!
//! ```no_run
//! use actix_web::{web, App, HttpServer};
//! use opentelemetry::global;
//! use opentelemetry_instrumentation_actix_web::RequestTracing;
//! use opentelemetry_sdk::trace::SdkTracerProvider;
//!
//! async fn index() -> &'static str {
//!     "Hello world!"
//! }
//!
//! #[actix_web::main]
//! async fn main() -> std::io::Result<()> {
//!     // Swap for `opentelemetry_otlp` or any other compatible
//!     // exporter to send metrics to your collector.
//!     let exporter = opentelemetry_stdout::SpanExporter::default();
//!
//!     // Configure your tracer provider with your exporter(s)
//!     let provider = SdkTracerProvider::builder()
//!         .with_simple_exporter(exporter)
//!         .build();
//!     global::set_tracer_provider(provider);
//!
//!     // add the request tracing middleware to create spans for each request
//!     HttpServer::new(|| {
//!         App::new()
//!             .wrap(RequestTracing::new())
//!             .service(web::resource("/").to(index))
//!     })
//!     .bind("127.0.0.1:8080")?
//!     .run()
//!     .await
//! }
//! ```
//!
//! Request metrics middleware (requires the `metrics` feature):
//!
//! ```no_run
//! use actix_web::{dev, http, web, App, HttpRequest, HttpServer};
//! use opentelemetry::{global, KeyValue};
//! # #[cfg(feature = "metrics")]
//! use opentelemetry_instrumentation_actix_web::{RequestMetrics, RequestTracing};
//! use opentelemetry_sdk::{metrics::SdkMeterProvider, Resource};
//!
//! async fn index() -> &'static str {
//!     "Hello world!"
//! }
//!
//! # #[cfg(feature = "metrics")]
//! #[actix_web::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Swap for `opentelemetry_otlp` or any other compatible
//!     // exporter to send metrics to your collector.
//!     let exporter = opentelemetry_stdout::MetricExporter::default();
//!
//!     // set up your meter provider with your exporter(s)
//!     let provider = SdkMeterProvider::builder()
//!         .with_periodic_exporter(exporter)
//!         .with_resource(
//!             Resource::builder_empty()
//!                 .with_attribute(KeyValue::new("service.name", "my_app"))
//!                 .build(),
//!         )
//!         .build();
//!     global::set_meter_provider(provider.clone());
//!
//!     // Run actix server, metrics are now available at http://localhost:8080/metrics
//!     HttpServer::new(move || {
//!         App::new()
//!             .wrap(RequestTracing::new())
//!             .wrap(RequestMetrics::default())
//!             .service(web::resource("/").to(index))
//!         })
//!         .bind("localhost:8080")?
//!         .run()
//!         .await;
//!
//!     //Shutdown the meter provider. This will trigger an export of all metrics.
//!     provider.shutdown()?;
//!
//!     Ok(())
//! }
//! # #[cfg(not(feature = "metrics"))]
//! # fn main() {}
//! ```
//!
//! For more information on how to configure Prometheus with [OTLP](https://prometheus.io/docs/guides/opentelemetry)
//!
//! ### Exporter configuration
//!
//! [`actix-web`] uses [`tokio`] as the underlying executor, so exporters should be
//! configured to be non-blocking:
//!
//! ```toml
//! [dependencies]
//! ## if exporting to jaeger, use the `tokio` feature.
//! opentelemetry-jaeger = { version = "..", features = ["rt-tokio-current-thread"] }
//!
//! ## if exporting to zipkin, use the `tokio` based `reqwest-client` feature.
//! opentelemetry-zipkin = { version = "..", features = ["reqwest-client"], default-features = false }
//!
//! ## ... ensure the same same for any other exporters
//! ```
//!
//! [`actix-web`]: https://crates.io/crates/actix-web
//! [`tokio`]: https://crates.io/crates/tokio
#![deny(missing_docs, unreachable_pub, missing_debug_implementations)]
#![cfg_attr(docsrs, feature(doc_cfg), deny(rustdoc::broken_intra_doc_links))]

#[cfg(feature = "awc")]
mod client;
mod middleware;
mod util;

#[cfg(feature = "awc")]
#[cfg_attr(docsrs, doc(cfg(feature = "awc")))]
pub use client::{ClientExt, InstrumentedClientRequest};

#[cfg(feature = "metrics")]
#[cfg_attr(docsrs, doc(cfg(feature = "metrics")))]
pub use middleware::metrics::{RequestMetrics, RequestMetricsBuilder, RequestMetricsMiddleware};
#[cfg(feature = "metrics")]
#[cfg_attr(docsrs, doc(cfg(feature = "metrics")))]
pub use util::metrics_attributes_from_request;

pub use {
    middleware::route_formatter::RouteFormatter,
    middleware::trace::{RequestTracing, RequestTracingMiddleware},
};
