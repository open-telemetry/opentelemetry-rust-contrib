# opentelemetry-stackdriver

[Documentation](https://docs.rs/opentelemetry-stackdriver/)

This crate provides an `opentelemetry` exporter for use with Google StackDriver trace. It uses gRPC to send tracing spans.

It is licensed under the Apache 2.0 license. Contributions are welcome.

## Propagator

Feature flag `propagator` will enable the `GoogleTraceContextPropagator` which implements the `TextMapPropagator` trait for Google `X-Cloud-Trace-Context` Trace Context format.

Example usage:

```rust
opentelemetry::global::set_text_map_propagator(GoogleTraceContextPropagator::new());
```
