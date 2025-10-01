# Changelog

## vNext

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
