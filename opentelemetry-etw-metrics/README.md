![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

# OpenTelemetry user_events Exporter

[![Crates.io: opentelemetry-user-events-metrics](https://img.shields.io/crates/v/opentelemetry-user-events-metrics.svg)](https://crates.io/crates/opentelemetry-user-events-metrics)

This crate contains OpenTelemetry metrics exporter to
[ETW (Event Tracing for Windows)](https://learn.microsoft.com/en-us/windows-hardware/drivers/devtest/event-tracing-for-windows--etw-), a Windows solution
for efficient tracing of kernel or application-defined events, similar to user-events on Linux.
ETW events can be consumed in real-time or from a log file.

ETW is included in Microsoft Windows 2000 and later.

This ETW exporter enables applications to use OpenTelemetry APIs to capture telemetry events and write to the ETW subsystem. From ETWs, the events can be
captured by agents running locally and listening for specific ETW events.
