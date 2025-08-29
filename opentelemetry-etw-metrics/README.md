# OpenTelemetry Metrics Exporter for ETW

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

| Status        |           |
| ------------- |-----------|
| Stability     | stable    |
| Owners        | [Cijo Thomas](https://github.com/cijothomas), [Lalit Kumar Bhasin](https://github.com/lalitb), [Matthew Boddewyn](https://github.com/mattbodd), [Pato Sandaña](https://github.com/psandana)|

This crate contains OpenTelemetry Metrics exporter to
[ETW (Event Tracing for Windows)](https://learn.microsoft.com/en-us/windows-hardware/drivers/devtest/event-tracing-for-windows--etw-), a Windows solution
for efficient tracing of kernel or application-defined events, similar to user-events on Linux.
ETW events can be consumed in real-time or from a log file.

ETW events created with this crate can be generated and collected on Windows Vista or later.

This ETW exporter enables applications to use OpenTelemetry APIs to capture telemetry events and write to the ETW subsystem. From ETWs, the events can be
captured by agents running locally and listening for specific ETW events.

[![Crates.io: opentelemetry-etw-metrics](https://img.shields.io/crates/v/opentelemetry-etw-metrics.svg)](https://crates.io/crates/opentelemetry-etw-metrics)
[![Documentation](https://docs.rs/opentelemetry-etw-metrics/badge.svg)](https://docs.rs/opentelemetry-etw-metrics)
[![LICENSE](https://img.shields.io/crates/l/opentelemetry-etw-metrics)](./LICENSE)
[![GitHub Actions CI](https://github.com/open-telemetry/opentelemetry-rust-contrib/workflows/CI/badge.svg)](https://github.com/open-telemetry/opentelemetry-rust-contrib/actions?query=workflow%3ACI+branch%3Amain)
[![Slack](https://img.shields.io/badge/slack-@cncf/otel/rust-brightgreen.svg?logo=slack)](https://cloud-native.slack.com/archives/C03GDP0H023)

## Viewing ETW Metrics

Metrics exported to ETW can be viewed using tools like `logman`, `perfview` etc. This crate currently uses
a hardcoded provided GUID `EDC24920-E004-40F6-A8E1-0E6E48F39D84`. The payload is binary encoded in `protobuf`
format, following the OTLP specification.

// TODO - add instructions.

### Tracelog
As a command line utility, a `tracelog` session can be run using the following commands:
- `tracelog -start MyTraceSession -f MyTraceFile.etl -guid #EDC24920-E004-40F6-A8E1-0E6E48F39D84`
- <run Rust code to emit ETW events>
- `tracelog -stop MyTraceSession`
- `tracefmt -o MyTraceFile.txt MyTraceFile.etl`
- `notepad MyTraceFile.txt`

The payload is protobuf encoded, and won't be human readable. // TODO: Show how
to decide the payload to human readable format.

## OpenTelemetry Overview

OpenTelemetry is an Observability framework and toolkit designed to create and
manage telemetry data such as traces, metrics, and logs. OpenTelemetry is
vendor- and tool-agnostic, meaning that it can be used with a broad variety of
Observability backends, including open source tools like [Jaeger] and
[Prometheus], as well as commercial offerings.

OpenTelemetry is *not* an observability backend like Jaeger, Prometheus, or other
commercial vendors. OpenTelemetry is focused on the generation, collection,
management, and export of telemetry. A major goal of OpenTelemetry is that you
can easily instrument your applications or systems, no matter their language,
infrastructure, or runtime environment. Crucially, the storage and visualization
of telemetry is intentionally left to other tools.