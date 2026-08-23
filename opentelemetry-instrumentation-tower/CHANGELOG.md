# Changelog

## vNext

### Added


* `http::server::LayerBuilder::with_tracing(bool)` and
  `http::server::LayerBuilder::with_metrics(bool)` to enable or disable each
  signal for a layer (both default to enabled). Disabling tracing does not stop
  context propagation: incoming trace headers are still extracted and the
  current context still flows to the inner service.
  [#679](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/679)
* The instrumentation scope now carries the OpenTelemetry semantic conventions
  schema URL.
  [#679](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/679)
* HTTP client instrumentation layer (`http::client::Layer`) producing a
  `SpanKind::Client` span and the standard `http.client.*` metrics, and
  injecting the current trace context into outgoing request headers.
  Tracing and metrics can be toggled per layer via `with_tracing(bool)` and
  `with_metrics(bool)` (both enabled by default).
  [#700](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/700)
* Cargo features to select which layers are compiled: `http-server` and
  `http-client` (both enabled by default).
  [#700](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/700)
* The **client** layer always defaults to `NoRouteExtractor` (method-only span
  names); the `axum` matched-path extractor only applies to server routing.
  [#700](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/700)
* `hyper-http-client` and `reqwest-http-client` examples demonstrating the
  client layer with Hyper and reqwest clients via `tower-reqwest`, with OTLP
  export.
  [#700](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/700)

### Changed

* **BREAKING**: Reorganized the public API into `http::server` and `http::client`
  modules with unprefixed `Layer`, `LayerBuilder`, `Service`, and `ResponseFuture`
  types, and moved the extractors into `http::extractors`. The `HTTPLayer` /
  `HTTPService` / `HTTPLayerBuilder` / `ResponseFuture` types introduced in
  v0.18.0 are replaced by
  `http::server::{Layer, Service, ResponseFuture, LayerBuilder}`.
  [#717](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/717)

### Migration Guide

#### Type and module changes

The former flat, `HTTP*`-prefixed types now live under `http::server`:

- `HTTPLayer` → `http::server::Layer`
- `HTTPLayerBuilder` → `http::server::LayerBuilder`
- `HTTPService` → `http::server::Service`
- `ResponseFuture` → `http::server::ResponseFuture`

Route and attribute extractors moved to `http::extractors`.

Before:

```rust
use opentelemetry_instrumentation_tower::HTTPLayerBuilder;

let layer = HTTPLayerBuilder::builder().build().unwrap();
```

After:

```rust
use opentelemetry_instrumentation_tower::http::server;

let layer = server::Layer::new();
```

#### Route extraction configuration

```rust
use opentelemetry_instrumentation_tower::http::{
    extractors::{NoRouteExtractor, PathExtractor},
    server::LayerBuilder,
};

// No route (default without axum feature) - span name: "GET"
let layer = LayerBuilder::builder()
    .with_route_extractor(NoRouteExtractor)
    .build()
    .unwrap();

// Path (strips query params) - span name: "GET /users/123"
let layer = LayerBuilder::builder()
    .with_route_extractor(PathExtractor)
    .build()
    .unwrap();

// Custom function - return Some(route) or None for method-only
let layer = LayerBuilder::builder()
    .with_route_extractor_fn(|req: &http::Request<_>| {
        Some(req.uri().path().to_owned())
    })
    .build()
    .unwrap();
```

## v0.18.0

Released 2026-Jul-28

### Added

* Added OpenTelemetry trace support
  [#431](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/431)
* Configurable route extraction with built-in extractors:
  [#528](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/528)
  - `NoRouteExtractor` - No route, safest for cardinality
  - `PathExtractor` - Uses the URL path without query params (e.g., `/users/123`)
  - `AxumMatchedPathExtractor` - Uses Axum's `MatchedPath` for route templates (requires `axum` feature)
  - `FnRouteExtractor` - Custom function-based extraction via `with_route_extractor_fn()`
* Default route extractor depends on features:
  - With `axum` feature: Uses `AxumMatchedPathExtractor` (route templates, low cardinality)
  - Without `axum` feature: Uses `NoRouteExtractor` (safest)
* If a route is extracted, the same value is used for both the `http.route` attribute and the tracing span name. If no route is extracted, the span name is the HTTP method only, per semantic conventions.

### Changed

* **BREAKING**: Removed public `with_meter()` method. The middleware now uses global meter and tracer providers by
  default via `opentelemetry::global::meter()` and `opentelemetry::global::tracer()`. The `with_meter()` method is
  retained as a non-public test utility to allow injecting custom meters without relying on global state.
  [#431](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/431)
* **BREAKING**: Renamed types. Use the new names:
    - `HTTPMetricsLayer` → `HTTPLayer`
    - `HTTPMetricsService` → `HTTPService`
    - `HTTPMetricsResponseFuture` → `ResponseFuture`
    - `HTTPMetricsLayerBuilder` → `HTTPLayerBuilder`
  [#431](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/431)
  [#714](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/714)
* **BREAKING**: Update default  `http.server.request.duration` histogram boundaries to OTel semantic conventions.
  [#525](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/525)
* **BREAKING**: Remove `with_request_duration_bounds` builder method.
  Alternate histogram bucket boundaries can be applied with the standard OpenTelemetry Views; see `examples` directory in crate for usage.
  [#525](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/525)

### Migration Guide

#### Route Extraction Configuration

```rust
use opentelemetry_instrumentation_tower::{
    HTTPLayerBuilder,
    NoRouteExtractor,
    PathExtractor,
};

// No route (default without axum feature) - span name: "GET"
let layer = HTTPLayerBuilder::builder()
    .with_route_extractor(NoRouteExtractor)
    .build()
    .unwrap();

// Path (strips query params) - span name: "GET /users/123"
let layer = HTTPLayerBuilder::builder()
    .with_route_extractor(PathExtractor)
    .build()
    .unwrap();

// Custom function - return Some(route) or None for method-only
let layer = HTTPLayerBuilder::builder()
    .with_route_extractor_fn(|req: &http::Request<_>| {
        Some(req.uri().path().to_owned())
    })
    .build()
    .unwrap();
```

#### API Changes

Before:

```rust
use opentelemetry_instrumentation_tower::HTTPMetricsLayerBuilder;

let layer = HTTPMetricsLayerBuilder::builder()
.with_meter(meter)
.build()
.unwrap();
```

After:

```rust
use opentelemetry_instrumentation_tower::HTTPLayer;

// Set global providers
global::set_meter_provider(meter_provider);
global::set_tracer_provider(tracer_provider); // for tracing support

// Then create the layer - simple API using global providers
let layer = HTTPLayer::new();
```

#### Type Name Changes

- Replace `HTTPMetricsLayerBuilder` with `HTTPLayerBuilder`
- Replace `HTTPMetricsLayer` with `HTTPLayer`
- Replace `HTTPMetricsService` with `HTTPService`
- Replace `HTTPMetricsResponseFuture` with `ResponseFuture`

## v0.17.0

### Changed

* Update to OpenTelemetry v0.31
  [#456](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/456)
* Migrate to use `opentelemetry-semantic-conventions` package for metric names and attribute keys instead of hardcoded
  strings
  [#435](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/435)
* Add dependency on otel semantic conventions crate and use constants from it instead of hardcoded attribute names. The
  values are unchanged
    - `HTTP_SERVER_ACTIVE_REQUESTS_METRIC` now uses `semconv::metric::HTTP_SERVER_ACTIVE_REQUESTS`
    - `HTTP_SERVER_REQUEST_BODY_SIZE_METRIC` now uses `semconv::metric::HTTP_SERVER_REQUEST_BODY_SIZE`
    - `HTTP_SERVER_RESPONSE_BODY_SIZE_METRIC` now uses `semconv::metric::HTTP_SERVER_RESPONSE_BODY_SIZE`
    - `HTTP_SERVER_DURATION_METRIC` now uses `semconv::metric::HTTP_SERVER_REQUEST_DURATION`
* Update attribute keys to use semantic conventions constants:
    - `NETWORK_PROTOCOL_NAME_LABEL` now uses `semconv::attribute::NETWORK_PROTOCOL_NAME`
    - `HTTP_REQUEST_METHOD_LABEL` now uses `semconv::attribute::HTTP_REQUEST_METHOD`
    - `HTTP_ROUTE_LABEL` now uses `semconv::attribute::HTTP_ROUTE`
    - `HTTP_RESPONSE_STATUS_CODE_LABEL` now uses `semconv::attribute::HTTP_RESPONSE_STATUS_CODE`

### Added

* Add comprehensive test coverage for all HTTP server metrics with attribute validation
  [#435](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/435)

## v0.16.0

Initial release of OpenTelemetry Tower instrumentation middleware for HTTP metrics collection.
[#248](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/248)

### Added

* HTTP server metrics middleware for Tower-compatible services
* Support for Axum framework via `axum` feature flag
* Metrics collection for:
    - `http.server.request.duration` - Request duration histogram
    - `http.server.active_requests` - Active requests counter
    - `http.server.request.body.size` - Request body size histogram
    - `http.server.response.body.size` - Response body size histogram
* Configurable request duration histogram boundaries
* Custom request and response attribute extractors
* Automatic protocol version, HTTP method, URL scheme, and status code labeling
* Route extraction for Axum applications
