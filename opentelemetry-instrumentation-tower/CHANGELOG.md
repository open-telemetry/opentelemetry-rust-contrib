# Changelog

## vNext

### Added

* Configurable route extraction with built-in extractors:
  - `NoRouteExtractor` - No route, uses only HTTP method (e.g., `GET`), safest for cardinality
  - `PathExtractor` - Uses the URL path without query params (e.g., `/users/123`)
  - `NormalizedPathExtractor` - Normalizes numeric IDs to `{id}` and UUIDs to `{uuid}` (requires `uuid` feature)
  - `AxumMatchedPathExtractor` - Uses Axum's `MatchedPath` for route templates (requires `axum` feature)
  - `FnRouteExtractor` - Custom function-based extraction via `with_route_extractor_fn()`
* New `uuid` feature flag for UUID detection in `NormalizedPathExtractor`
* Default route extractor depends on features:
  - With `axum` feature: Uses `AxumMatchedPathExtractor` (route templates, low cardinality)
  - Without `axum` feature: Uses `NoRouteExtractor` (method only, safest)
* Route extraction now provides both span names and `http.route` metric attribute from the same source

### Changed

* **BREAKING**: Removed public `with_meter()` method. The middleware now uses global meter and tracer providers by
  default via `opentelemetry::global::meter()` and `opentelemetry::global::tracer()`. The `with_meter()` method is
  retained as a non-public test utility to allow injecting custom meters without relying on global state.
* **BREAKING**: Renamed types. Use the new names:
    - `HTTPMetricsLayer` → `HTTPLayer`
    - `HTTPMetricsService` → `HTTPService`
    - `HTTPMetricsResponseFuture` → `HTTPResponseFuture`
    - `HTTPMetricsLayerBuilder` → `HTTPLayerBuilder`
* Added OpenTelemetry trace support
* **BREAKING**: Update default  `http.server.request.duration` histogram boundaries to OTel semantic conventions.
* **BREAKING**: Remove `with_request_duration_bounds` builder method.
  Alternate histogram bucket boundaries can be applied with the standard OpenTelemetry Views; see `examples` directory in crate for usage.

### Migration Guide

#### Route Extraction Configuration

```rust
use opentelemetry_instrumentation_tower::{
    HTTPLayerBuilder,
    NoRouteExtractor,
    PathExtractor,
    NormalizedPathExtractor,
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

// Normalized path (replaces IDs with {id}, UUIDs with {uuid}) - span name: "GET /users/{id}"
let layer = HTTPLayerBuilder::builder()
    .with_route_extractor(NormalizedPathExtractor)
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
- Replace `HTTPMetricsResponseFuture` with `HTTPResponseFuture`

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
