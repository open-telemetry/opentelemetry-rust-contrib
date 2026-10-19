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
parameter that the conventions name with `REDACTED`, and keeps the key:
`fields=name&sig=REDACTED`.

The default keys are the ones that the
[`url.query` conventions](https://opentelemetry.io/docs/specs/semconv/registry/attributes/url/#url-query)
list:

- `X-Amz-Signature`
- `X-Amz-Credential`
- `X-Amz-Security-Token`
- `AWSAccessKeyId`
- `Signature`
- `sig`
- `X-Goog-Signature`

Use `http::server::LayerBuilder::with_sensitive_query_parameters` to name the
keys yourself. The list replaces the default one, it does not extend it, and an
empty list disables redaction. Keys are matched case-sensitively.

## Reported errors

The server span carries `error.type` when a request ends with an error:

- A 5xx response sets `error.type` to the status code number as a string, for
  example `"500"`.
- An error from the inner service, returned before a status code exists, sets
  `error.type` to the Rust type name of that error, for example
  `"my_crate::MyError"`, obtained with `std::any::type_name`.
- A 2xx, 3xx or 4xx response carries no `error.type`. Per the HTTP span status
  rules, a 4xx is not a server error.

The middleware wraps an arbitrary Tower `Service` and has no knowledge of your
application's error types, so it can only report the two classifications it
can produce on its own: the HTTP status code once one exists, or the error's
Rust type name before one does. A status code always wins when one is
available, and it never substitutes exception detail for a status code that 
is already present.

To report a domain-specific error, for example `"payment_declined"`, add your
own attribute with `http::server::LayerBuilder::with_response_extractor`. Do
not reuse the `error.type` key: on a 5xx response, the layer sets `error.type`
to the status code after extractors run, so it overwrites any `error.type` an
extractor sets, instead use `myapp.error.type`.

## Client address

The server span carries `client.address`, which the conventions define as the
original client address behind all proxies. The middleware reports the first
address it can find in this order:

- The first entry of the `X-Forwarded-For` header. A chain of proxies prepends
  one address per hop, so the first entry is the original client:
  `X-Forwarded-For: 203.0.113.195, 70.41.3.18` reports `203.0.113.195`.
- The node name of the first `for` directive in the `Forwarded` header, for
  example `192.0.2.43` for `Forwarded: for=192.0.2.43;by=203.0.113.60`.
- The host of the socket address of the immediate peer. A Tower `Service` does
  not see the socket, so this source needs the service in front to expose the
  peer address. With Axum, build the router with
  `Router::into_make_service_with_connect_info::<SocketAddr>()` and enable the
  `axum-connect-info` feature of this crate. Without the feature, or without a
  socket address in the request extensions, the attribute is not emitted.

Trust these headers only when a proxy in front of your service sets or strips
them. A client can send its own `X-Forwarded-For` value, and the middleware
cannot tell a proxy-added entry from a spoofed one.

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
