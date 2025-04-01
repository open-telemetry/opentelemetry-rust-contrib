# OpenTelemetry Log Exporter for Linux user_events

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

| Status        |           |
| ------------- |-----------|
| Stability     | beta      |
| Owners        | [Cijo Thomas](https://github.com/cijothomas), [Lalit Kumar Bhasin](https://github.com/lalitb) |

This crate contains a Log Exporter to export logs to Linux
[user_events](https://docs.kernel.org/trace/user_events.html), which is a
solution for user process tracing, similar to ETW (Event Tracing for Windows) on
Windows. It builds on top of the Linux Tracepoints, and so allows user processes
to create events and trace data that can be viewed via existing tools like
ftrace and perf.

This kernel feature is supported started in Linux kernel 6.4 onwards. The feature enables

- A faster path for tracing from user mode application utilizing kernel mode memory address space.
- User processes can now export telemetry events only when it is useful i.e, when the registered set of tracepoint events are enabled.

This user_events exporter enables applications to use OpenTelemetry API to capture the telemetry events, and write to user_events subsystem. From user_events, the events can be

- Captured by the agents running locally, and listening for specific events withing user_events subsystem.
- Or real-time monitoring using local Linux tool like [perf](https://perf.wiki.kernel.org/index.php/Main_Page) or ftrace.

[![Crates.io: opentelemetry-user-events-logs](https://img.shields.io/crates/v/opentelemetry-user-events-logs.svg)](https://crates.io/crates/opentelemetry-user-events-logs)
[![Documentation](https://docs.rs/opentelemetry-user-events-logs/badge.svg)](https://docs.rs/opentelemetry-user-events-logs)
[![LICENSE](https://img.shields.io/crates/l/opentelemetry-user-events-logs)](./LICENSE)
[![GitHub Actions CI](https://github.com/open-telemetry/opentelemetry-rust-contrib/workflows/CI/badge.svg)](https://github.com/open-telemetry/opentelemetry-rust-contrib/actions?query=workflow%3ACI+branch%3Amain)
[![Slack](https://img.shields.io/badge/slack-@cncf/otel/rust-brightgreen.svg?logo=slack)](https://cloud-native.slack.com/archives/C03GDP0H023)

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

[Prometheus]: https://prometheus.io
[Jaeger]: https://www.jaegertracing.io

