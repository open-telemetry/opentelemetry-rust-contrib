# OpenTelemetry Log Exporter for Linux user_events

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

This crate provides a Log Exporter to export logs to the systemd journal (journald) using the OpenTelemetry framework. The exporter supports both plain text and JSON formats for log entries and includes options for setting message size limits, identifiers, and attribute prefixes.

Journald is a system service for collecting and storing log data. It is a part of systemd, a suite of system management daemons, libraries, and utilities designed for Linux. This exporter allows OpenTelemetry to send log data directly to journald, enabling seamless integration with systemd's logging infrastructure.

This exporter requires libsystemd for sending log entries to the journald daemon.


This kernel feature is supported started in Linux kernel 5.18 onwards. The feature enables
 - A faster path for tracing from user mode application utilizing kernel mode memory address space. 
 - User processes can now export telemetry events only when it is useful i.e, when the registered set of tracepoint events are enabled.

 This user_events exporter enables applications to use OpenTelemetry API to capture the telemetry events, and write to user_events subsystem. From user_events, the events can be
  - Captured by the agents running locally, and listening for specific events withing user_events subsystem.
  - Or real-time monitoring using local Linux tool like [perf](https://perf.wiki.kernel.org/index.php/Main_Page) or ftrace.

[![Crates.io: opentelemetry-journald-logs](https://img.shields.io/crates/v/opentelemetry-journald-logs.svg)](https://crates.io/crates/opentelemetry-journald-logs)
[![Documentation](https://docs.rs/opentelemetry-journald-logs/badge.svg)](https://docs.rs/opentelemetry-journald-logs)
[![LICENSE](https://img.shields.io/crates/l/opentelemetry-journald-logs)](./LICENSE)
[![GitHub Actions CI](https://github.com/open-telemetry/opentelemetry-rust/workflows/CI/badge.svg)](https://github.com/open-telemetry/opentelemetry-rust/actions?query=workflow%3ACI+branch%3Amain)
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
