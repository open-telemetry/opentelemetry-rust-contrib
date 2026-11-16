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
as `http.request.header.<key>`, are not emitted by default. `client.port` has a
builder switch, see [Client address](#client-address). Add the other ones with
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

The server span carries `client.address`. The conventions define it as the
original client address behind all proxies, if it is known, and as the
immediate peer otherwise. Only your application knows which proxies it can
trust, so you pick the source with
`http::server::LayerBuilder::with_client_address_extractor`:

- `SocketAddressExtractor` is the default. It reports the host of the socket
  address of the immediate peer, which a client cannot spoof. A Tower `Service`
  does not see the socket, so the service in front must expose the peer
  address. With Axum, build the router with
  `Router::into_make_service_with_connect_info::<SocketAddr>()` and enable the
  `axum-connect-info` feature of this crate. Without the feature, or without a
  socket address in the request extensions, the attribute is not emitted.
- `ForwardedHeadersExtractor` reads the forwarded headers. It reports the first
  entry of `X-Forwarded-For`, then the first `for` directive of `Forwarded`,
  and falls back to the socket address. A chain of proxies appends one address
  per hop, so `X-Forwarded-For: 203.0.113.195, 70.41.3.18` reports
  `203.0.113.195`. The extractor removes the port and the IPv6 brackets, so
  `Forwarded: for="[2001:db8::1]:4711"` reports `2001:db8::1`.
- `with_client_address_extractor_fn` runs your own function. Use it to apply
  your trusted-proxy policy, for example with
  [`axum-client-ip`](https://docs.rs/axum-client-ip).

Use `ForwardedHeadersExtractor` only when a proxy in front of your service sets
or strips these headers. A client can send its own `X-Forwarded-For` value, and
the extractor cannot tell a proxy-added entry from a spoofed one.

```rust
use opentelemetry_instrumentation_tower::http::extractors::ForwardedHeadersExtractor;
use opentelemetry_instrumentation_tower::http::server::LayerBuilder;

let layer = LayerBuilder::builder()
    .with_client_address_extractor(ForwardedHeadersExtractor)
    .build()
    .unwrap();
```

The
[HTTP server span conventions](https://opentelemetry.io/docs/specs/semconv/http/http-spans/#http-server-span)
mark `client.port` as `Opt-In`. Enable it with
`http::server::LayerBuilder::with_client_port(true)`. The port comes from the
same source as `client.address`, so `X-Forwarded-For: 203.0.113.195:4711`
reports `4711` with `ForwardedHeadersExtractor`. When that source has no port,
the span carries no `client.port`, even when the socket address has one.

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
