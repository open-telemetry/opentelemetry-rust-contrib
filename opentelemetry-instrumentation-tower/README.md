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
Tonic, etc.). The middleware emits the standard `http.server.*` metrics and a
server span per request, following the OpenTelemetry
[HTTP semantic conventions](https://opentelemetry.io/docs/specs/semconv/http/).

## Implemented specification

The middleware implements these documents:

- [HTTP spans](https://opentelemetry.io/docs/specs/semconv/http/http-spans/),
  and the
  [span examples](https://opentelemetry.io/docs/specs/semconv/http/http-spans/#examples)
  in particular.
- [HTTP metrics](https://opentelemetry.io/docs/specs/semconv/http/http-metrics/).
- [Recording errors](https://opentelemetry.io/docs/specs/semconv/general/recording-errors/).
- [Attribute requirement levels](https://opentelemetry.io/docs/specs/semconv/general/attribute-requirement-level/).

The server span carries every `Required`, `Conditionally Required`, and
`Recommended` attribute that a Tower service can read. Three points need
attention:

- **Attributes of the original client request.** A reverse proxy rewrites the
  connection and the `Host` header. The middleware therefore reads
  `client.address`, `server.address`, `server.port`, and `url.scheme` from the
  [`Forwarded`](https://www.rfc-editor.org/rfc/rfc7239) header first, then from
  the `X-Forwarded-For`, `X-Forwarded-Host`, and `X-Forwarded-Proto` headers, and
  falls back to the connection. A client can send these headers too, so a server
  that no proxy protects can receive any value in them.
- **`url.scheme` without a proxy.** A server receives a request target that
  carries no scheme, so the middleware reports `http` unless a forwarding header
  states otherwise. A server that terminates TLS itself therefore reports `http`.
- **Peer address.** A Tower service reads the request only, which carries no
  connection. With the `axum` feature, the middleware reads the peer address from
  `ConnectInfo`, which Axum records when the application serves with
  [`Router::into_make_service_with_connect_info`](https://docs.rs/axum/latest/axum/struct.Router.html#method.into_make_service_with_connect_info).
  Without it, `client.address` holds a forwarded address only, and
  `network.peer.address` stays unset.

`Opt-In` attributes, such as `http.request.header.<key>` and `client.port`, are
not emitted. Add them with
`http::server::LayerBuilder::with_request_extractor`.

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
