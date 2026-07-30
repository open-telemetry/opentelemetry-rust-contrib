# Tower OpenTelemetry HTTP Instrumentation Middleware

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

| Status    |                                              |
|-----------|----------------------------------------------|
| Stability | alpha                                        |
| Owners    | [Franco Posa](https://github.com/francoposa), [Jan Steinke](https://github.com/jan-xyz) |

[![Crates.io](https://img.shields.io/crates/v/opentelemetry-instrumentation-tower.svg)](https://crates.io/crates/opentelemetry-instrumentation-tower)
[![Documentation](https://docs.rs/opentelemetry-instrumentation-tower/badge.svg)](https://docs.rs/opentelemetry-instrumentation-tower)
[![License](https://img.shields.io/crates/l/opentelemetry-instrumentation-tower)](./LICENSE)
[![Slack](https://img.shields.io/badge/slack-@cncf/otel/rust-brightgreen.svg?logo=slack)](https://cloud-native.slack.com/archives/C03GDP0H023)

[OpenTelemetry](https://opentelemetry.io/) HTTP metrics and tracing middleware
for [Tower](https://docs.rs/tower)-compatible Rust HTTP servers and clients
(Axum, Hyper, reqwest, etc.). The middleware emits the standard `http.server.*`
and `http.client.*` metrics and a span per request, following the OpenTelemetry
[HTTP semantic conventions](https://opentelemetry.io/docs/specs/semconv/http/).

## Quick start

With the default `axum` feature, applying the server middleware is a single
layer call:

```rust
use axum::{routing::get, Router};
use opentelemetry_instrumentation_tower::http;

let app: Router = Router::new()
    .route("/", get(|| async { "hello" }))
    // Apply *after* the routes so the matched route template is available.
    .layer(http::server::Layer::new());
```

Instrument an outgoing client stack:

```rust
use opentelemetry_instrumentation_tower::http;
use tower::ServiceBuilder;

let client = ServiceBuilder::new()
    .layer(http::client::Layer::new())
    .service(inner_client);
```

See the [API documentation](https://docs.rs/opentelemetry-instrumentation-tower)
for emitted metrics, span attributes, customization via the layer builders, and
cardinality guidance.

## Examples

Runnable examples (Axum, Hyper, custom route extractor) live in the
[`examples/`](./examples) directory. They pin specific OpenTelemetry crate
versions and may need adjustments when those versions change.

## Changelog

See [CHANGELOG.md](./CHANGELOG.md) for release history.
