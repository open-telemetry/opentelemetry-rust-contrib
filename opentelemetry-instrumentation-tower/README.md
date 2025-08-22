# Tower OTEL HTTP Instrumentation Middleware

OpenTelemetry HTTP Metrics and Tracing Middleware for Tower-compatible Rust HTTP servers.

This middleware provides both metrics and distributed tracing for HTTP requests, following OpenTelemetry semantic conventions.

## Features

- **HTTP Metrics**: Request duration, active requests, request/response body sizes
- **Distributed Tracing**: HTTP spans with semantic attributes
- **Semantic Conventions**: Uses OpenTelemetry semantic conventions for consistent attribute naming
- **Flexible Configuration**: Support for custom attribute extractors and tracer configuration
- **Framework Support**: Works with any Tower-compatible HTTP framework (Axum, Hyper, etc.)

## Usage

### Basic Usage (Metrics Only)

```rust
use opentelemetry_instrumentation_tower::HTTPLayerBuilder;

let meter = global::meter("my-service");
let http_layer = HTTPLayerBuilder::builder()
    .with_meter(meter)
    .build()?;

let app = Router::new()
    .route("/", get(handler))
    .layer(http_layer);
```

### With Tracing

```rust
use opentelemetry_instrumentation_tower::HTTPLayerBuilder;

let meter = global::meter("my-service");
let http_layer = HTTPLayerBuilder::builder()
    .with_meter(meter)
    .with_tracing() // Uses global tracer provider
    .build()?;

let app = Router::new()
    .route("/", get(handler))
    .layer(http_layer);
```

### With Custom Tracer

```rust
use opentelemetry_instrumentation_tower::HTTPLayerBuilder;

let meter = global::meter("my-service");
let tracer = global::tracer_provider().tracer("my-service");

let http_layer = HTTPLayerBuilder::builder()
    .with_meter(meter)
    .with_tracer(tracer)
    .build()?;

let app = Router::new()
    .route("/", get(handler))
    .layer(http_layer);
```

### With Custom Attribute Extractors

```rust
use opentelemetry_instrumentation_tower::HTTPLayerBuilder;

let http_layer = HTTPLayerBuilder::builder()
    .with_meter(meter)
    .with_tracing()
    .with_request_extractor_fn(|req| {
        vec![KeyValue::new("custom.header", 
             req.headers().get("x-custom").unwrap_or_default().to_str().unwrap_or(""))]
    })
    .build()?;
```

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
- `http.target` - Request target (path + query)
- `url.full` - Full URL
- `http.route` - Matched route pattern (when available)
- `server.address` - Server host
- `user_agent.original` - User agent string
- `http.response.status_code` - HTTP response status code

## Examples

See `examples` directory in repo for runnable code and supporting config files.
Attempts are made to keep the code here synced, but it will not be perfect.

OTEL libraries in particular are sensitive to minor version changes at this point,
so the examples may only work with the OTEL crate versions pinned in `examples`.

Created by Franco Posa (franco @ [francoposa.io](https://francoposa.io))
