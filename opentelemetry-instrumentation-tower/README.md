# Tower OTEL HTTP Instrumentation Middleware

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

| Status    |                                              |
|-----------|----------------------------------------------|
| Stability | alpha                                        |
| Owners    | [Franco Posa](https://github.com/francoposa) |

OpenTelemetry HTTP Metrics and Tracing Middleware for Tower-compatible Rust HTTP servers.

This middleware provides both metrics and distributed tracing for HTTP requests, following OpenTelemetry semantic conventions.

## Features

- **HTTP Metrics**: Request duration, active requests, request/response body sizes
- **Distributed Tracing**: HTTP spans with semantic attributes
- **Semantic Conventions**: Uses OpenTelemetry semantic conventions for consistent attribute naming
- **Flexible Configuration**: Support for custom attribute extractors and tracer configuration
- **Framework Support**: Works with any Tower-compatible HTTP framework (Axum, Hyper, Tonic etc.)

## Metrics

The middleware exports the following metrics:

- `http.server.request.duration` - Duration of HTTP requests
- `http.server.active_requests` - Number of active HTTP requests
- `http.server.request.body.size` - Size of HTTP request bodies
- `http.server.response.body.size` - Size of HTTP response bodies

## Tracing

HTTP spans are created with the following attributes (following OpenTelemetry semantic conventions):

- `http.request.method` - HTTP method
- `url.scheme` - URL scheme (http/https)
- `url.path` - Request path
- `url.full` - Full URL
- `user_agent.original` - User agent string
- `http.response.status_code` - HTTP response status code

## Recommended Usage

### Histogram Bucket Boundaries

This library defaults to the OpenTelemetry semantic conventions for `http.server.request.duration` bucket boundaries:
`[ 0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1, 2.5, 5, 7.5, 10 ]` with seconds as the unit.

These boundaries do not capture durations over 10 seconds, which may be limiting for an http server.

To capture longer requests with some rough granularity on the upper end, the library also exports an alternate constant:

```rust
pub const ALTERNATE_HTTP_SERVER_DURATION_BOUNDS: [f64; 14] = [
    0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0,
];
```

The alternate constant or any other custom boundaries may be applied via the builder interface:

```rust
use opentelemetry_instrumentation_tower::{
    HTTPLayerBuilder, ALTERNATE_HTTP_SERVER_DURATION_BOUNDS,
};

// ...

fn main() {
    let otel_service_layer = HTTPLayerBuilder::builder()
        .with_request_duration_bounds(Vec::from(ALTERNATE_HTTP_SERVER_DURATION_BOUNDS))
        .build()
        .unwrap();
    // ...
}
```

## Examples

See `examples` directory in repo for runnable code and supporting config files.
Attempts are made to keep the code here synced, but it will not be perfect.

OTEL libraries in particular are sensitive to minor version changes at this point,
so the examples may only work with the OTEL crate versions pinned in `examples`.

Created by Franco Posa (franco @ [francoposa.io](https://francoposa.io))
