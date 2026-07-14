# OpenTelemetry Stackdriver

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

> **⚠️ DEPRECATED** — This crate is deprecated and will be removed from this
> repository. It has been unmaintained, and no committed maintainer stepped up
> in response to [issue #609]. Users should migrate to OTLP — Google Cloud
> supports OTLP ingestion directly, and the OpenTelemetry Collector ships a
> [`googlecloud` exporter]. The last release of this crate on crates.io will
> remain available, but no further releases are planned.
>
> [issue #609]: https://github.com/open-telemetry/opentelemetry-rust-contrib/issues/609
> [`googlecloud` exporter]: https://github.com/open-telemetry/opentelemetry-collector-contrib/tree/main/exporter/googlecloudexporter

| Status        |                |
| ------------- |----------------|
| Stability     | deprecated     |
| Owners        | _unmaintained_ |

This crate provides an `opentelemetry` exporter for use with Google Stackdriver trace. It uses gRPC to send tracing spans.
Contributions are welcome.

[![Crates.io: opentelemetry-stackdriver](https://img.shields.io/crates/v/opentelemetry-stackdriver.svg)](https://crates.io/crates/opentelemetry-stackdriver)
[![Documentation](https://docs.rs/opentelemetry-stackdriver/badge.svg)](https://docs.rs/opentelemetry-stackdriver)
[![Slack](https://img.shields.io/badge/slack-@cncf/otel/rust-brightgreen.svg?logo=slack)](https://cloud-native.slack.com/archives/C03GDP0H023)

## Propagator

Feature flag `propagator` will enable the `GoogleTraceContextPropagator` which implements the `TextMapPropagator` trait for Google `X-Cloud-Trace-Context` Trace Context format.

Example usage:

```rust
opentelemetry::global::set_text_map_propagator(GoogleTraceContextPropagator::new());
```
