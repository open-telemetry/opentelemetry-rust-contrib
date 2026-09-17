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
for [Tower](https://docs.rs/tower)-compatible Rust HTTP servers (Axum, Hyper,
Tonic, etc.). The middleware emits the standard `http.server.*` metrics, which
follow the
[HTTP metrics](https://opentelemetry.io/docs/specs/semconv/http/http-metrics/)
conventions, and a server span per request, which follows the
[HTTP spans](https://opentelemetry.io/docs/specs/semconv/http/http-spans/)
conventions. The goal is every `Required`, `Conditionally Required`, and
`Recommended` attribute that a Tower service can read. `Opt-In` attributes, such
as `http.request.header.<key>`, are not emitted. Add them with
`http::server::LayerBuilder::with_request_extractor`.

## Query string redaction

The `url.query` attribute holds the query string of the request, which can carry
a credential. The middleware therefore replaces the value of every query
parameter that the conventions name, such as `sig` or `X-Amz-Signature`, with
`REDACTED`, and keeps the key: `fields=name&sig=REDACTED`.

Use `http::server::LayerBuilder::with_sensitive_query_parameters` to name the
keys yourself. The list replaces the default one, it does not extend it, and an
empty list disables redaction. Keys are matched case-sensitively.

## Quick start

With the default `axum` feature, applying the middleware is a single layer call:

```rust
use axum::{routing::get, Router};
use opentelemetry_instrumentation_tower::http;

let app: Router = Router::new()
    .route("/", get(|| async { "hello" }))
    // Apply *after* the routes so the matched route template is available.
    .layer(http::server::Layer::new());
```

See the [API documentation](https://docs.rs/opentelemetry-instrumentation-tower)
for emitted metrics, span attributes, customization via
`http::server::LayerBuilder`, and cardinality guidance.

## Examples

Runnable examples (Axum, Hyper, custom route extractor) live in the
[`examples/`](./examples) directory. They pin specific OpenTelemetry crate
versions and may need adjustments when those versions change.

## Changelog

See [CHANGELOG.md](./CHANGELOG.md) for release history.
