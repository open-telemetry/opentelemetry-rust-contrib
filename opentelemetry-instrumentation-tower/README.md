# Tower OTEL HTTP Instrumentation Middleware

OpenTelemetry HTTP Metrics and Tracing Middleware for Tower-compatible Rust HTTP servers.

This middleware provides both metrics and distributed tracing for HTTP requests, following OpenTelemetry semantic conventions.

## Features

- **HTTP Metrics**: Request duration, active requests, request/response body sizes
- **Distributed Tracing**: HTTP spans with semantic attributes
- **Semantic Conventions**: Uses OpenTelemetry semantic conventions for consistent attribute naming
- **Flexible Configuration**: Support for custom attribute extractors and tracer configuration
- **Framework Support**: Works with any Tower-compatible HTTP framework (Axum, Hyper, Tonic etc.)

## Usage

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

## Examples

See `examples` directory in repo for runnable code and supporting config files.
Attempts are made to keep the code here synced, but it will not be perfect.

OTEL libraries in particular are sensitive to minor version changes at this point,
so the examples may only work with the OTEL crate versions pinned in `examples`.

Created by Franco Posa (franco @ [francoposa.io](https://francoposa.io))
