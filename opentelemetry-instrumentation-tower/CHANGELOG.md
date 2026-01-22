# Changelog

## vNext

### Added

* Configurable span name extraction with built-in extractors:
  - `MethodOnlySpanNameExtractor` - Uses only HTTP method (e.g., `GET`), safest for cardinality
  - `MethodAndPathSpanNameExtractor` - Uses method and path without query params (e.g., `GET /users/123`)
  - `NormalizedPathSpanNameExtractor` - Normalizes numeric IDs to `{id}` and UUIDs to `{uuid}` (requires `uuid` feature)
  - `AxumMatchedPathSpanNameExtractor` - Uses Axum's `MatchedPath` for route templates (requires `axum` feature)
  - `FnSpanNameExtractor` - Custom function-based extraction via `with_span_name_extractor_fn()`
* New `uuid` feature flag for UUID detection in `NormalizedPathSpanNameExtractor`
* Default span name extractor depends on features:
  - With `axum` feature: Uses `AxumMatchedPathSpanNameExtractor` (route templates, low cardinality)
  - Without `axum` feature: Uses `MethodOnlySpanNameExtractor` (method only, safest)

### Changed

* **BREAKING**: Removed public `with_meter()` method. The middleware now uses global meter and tracer providers by default via `opentelemetry::global::meter()` and `opentelemetry::global::tracer()`. The `with_meter()` method is retained as a non-public test utility to allow injecting custom meters without relying on global state.
* **BREAKING**: Renamed types. Use the new names:
  - `HTTPMetricsLayer` → `HTTPLayer`
  - `HTTPMetricsService` → `HTTPService`
  - `HTTPMetricsResponseFuture` → `HTTPResponseFuture`
  - `HTTPMetricsLayerBuilder` → `HTTPLayerBuilder`
* Added OpenTelemetry trace support

### Migration Guide

#### Span Name Configuration

```rust
use opentelemetry_instrumentation_tower::{
    HTTPLayerBuilder,
    MethodOnlySpanNameExtractor,
    MethodAndPathSpanNameExtractor,
    NormalizedPathSpanNameExtractor,
};

// Method only (default without axum feature)
let layer = HTTPLayerBuilder::builder()
    .with_span_name_extractor(MethodOnlySpanNameExtractor)
    .build()
    .unwrap();

// Method and path (strips query params)
let layer = HTTPLayerBuilder::builder()
    .with_span_name_extractor(MethodAndPathSpanNameExtractor)
    .build()
    .unwrap();

// Normalized path (replaces IDs with {id}, UUIDs with {uuid})
let layer = HTTPLayerBuilder::builder()
    .with_span_name_extractor(NormalizedPathSpanNameExtractor)
    .build()
    .unwrap();

// Custom function
let layer = HTTPLayerBuilder::builder()
    .with_span_name_extractor_fn(|req: &http::Request<_>| {
        format!("{} {}", req.method(), req.uri().path())
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
* Migrate to use `opentelemetry-semantic-conventions` package for metric names and attribute keys instead of hardcoded strings
* Add dependency on otel semantic conventions crate and use constants from it instead of hardcoded attribute names. The values are unchanged
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
