# Changelog

## vNext

### Added

* HTTP client instrumentation layer (`http::client::Layer`) producing a
  `SpanKind::Client` span and the standard `http.client.*` metrics, and
  injecting the current trace context into outgoing request headers.
  Tracing and metrics can be toggled per layer via `with_tracing(bool)` and
  `with_metrics(bool)` (both enabled by default).
* Cargo features to select which layers are compiled: `http-server` and
  `http-client` (both enabled by default).
* Configurable route extraction with built-in extractors:
  - `NoRouteExtractor` - No route, uses only HTTP method (e.g., `GET`), safest for cardinality
  - `PathExtractor` - Uses the URL path without query params (e.g., `/users/123`)
  - `AxumMatchedPathExtractor` - Uses Axum's `MatchedPath` for route templates (requires `axum` feature)
  - `FnRouteExtractor` - Custom function-based extraction via `with_route_extractor_fn()`
* Default route extractor depends on features:
  - With `axum` feature: Uses `AxumMatchedPathExtractor` (route templates, low cardinality)
  - Without `axum` feature: Uses `NoRouteExtractor` (method only, safest)
  - The **client** layer always defaults to `NoRouteExtractor` (method-only span names); the
    `axum` matched-path extractor only applies to server routing.
* Route extraction now provides both span names and `http.route` metric attribute from the same source
* Distributed tracing for the HTTP server layer (`SpanKind::Server` spans), in addition to the existing HTTP server metrics

### Changed

* **BREAKING**: Reorganized the public API into `http::server` and `http::client`
  modules with unprefixed `Layer`, `LayerBuilder`, `Service`, and `ResponseFuture`
  types, and moved the extractors into `http::extractors`. The previous
  `HTTPMetricsLayer` / `HTTPMetricsService` / `HTTPMetricsResponseFuture` /
  `HTTPMetricsLayerBuilder` types are replaced by
  `http::server::{Layer, Service, ResponseFuture, LayerBuilder}`.
* **BREAKING**: Removed the public `with_meter()` builder method. The layers now
  use the global meter and tracer providers via `opentelemetry::global::meter()`
  and `opentelemetry::global::tracer()`.
* **BREAKING**: Updated the default `http.server.request.duration` histogram
  boundaries to the OpenTelemetry semantic-conventions defaults.
* **BREAKING**: Removed the `with_request_duration_bounds` builder method.
  Customize histogram boundaries with OpenTelemetry Views instead; see the
  `examples` directory in the crate for usage.

### Migration Guide

#### Type and module changes

The former flat, `HTTPMetrics*`-prefixed types now live under `http::server`:

- `HTTPMetricsLayer` → `http::server::Layer`
- `HTTPMetricsLayerBuilder` → `http::server::LayerBuilder`
- `HTTPMetricsService` → `http::server::Service`
- `HTTPMetricsResponseFuture` → `http::server::ResponseFuture`

Route and attribute extractors moved to `http::extractors`.

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
use opentelemetry_instrumentation_tower::http::server;

// Configure the global providers once, before building the layer.
global::set_meter_provider(meter_provider);
global::set_tracer_provider(tracer_provider); // for tracing support

// Simplest form — reads the global providers.
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

## v0.17.0

### Changed

* Update to OpenTelemetry v0.31
* Migrate to use `opentelemetry-semantic-conventions` package for metric names and attribute keys instead of hardcoded
  strings
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

## v0.16.0

Initial release of OpenTelemetry Tower instrumentation middleware for HTTP metrics collection.

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
