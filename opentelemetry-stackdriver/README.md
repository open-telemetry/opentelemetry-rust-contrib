# OpenTelemetry Stackdriver

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

| Status        |           |
| ------------- |-----------|
| Stability     | alpha     |
| Owners        | TBD       |

This crate provides an `opentelemetry` exporter for use with Google Stackdriver trace. It uses gRPC to send tracing spans.
Contributions are welcome.

[![Crates.io: opentelemetry-stackdriver](https://img.shields.io/crates/v/opentelemetry-stackdriver.svg)](https://crates.io/crates/opentelemetry-stackdriver)
[![Documentation](https://docs.rs/opentelemetry-stackdriver/badge.svg)](https://docs.rs/opentelemetry-stackdriver)
[![LICENSE](https://img.shields.io/crates/l/opentelemetry-stackdriver)](./LICENSE)
[![GitHub Actions CI](https://github.com/open-telemetry/opentelemetry-rust-contrib/workflows/CI/badge.svg)](https://github.com/open-telemetry/opentelemetry-rust-contrib/actions?query=workflow%3ACI+branch%3Amain)
[![Slack](https://img.shields.io/badge/slack-@cncf/otel/rust-brightgreen.svg?logo=slack)](https://cloud-native.slack.com/archives/C03GDP0H023)

## Propagator

Feature flag `propagator` will enable the `GoogleTraceContextPropagator` which implements the `TextMapPropagator` trait for Google `X-Cloud-Trace-Context` Trace Context format.

Example usage:

```rust
opentelemetry::global::set_text_map_propagator(GoogleTraceContextPropagator::new());
```
