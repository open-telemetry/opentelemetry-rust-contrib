![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust-contrib/main/assets/logo-text.png

# OpenTelemetry ETW Exporter

[![Crates.io: opentelemetry-etw-metrics](https://img.shields.io/crates/v/opentelemetry-etw-metrics.svg)](https://crates.io/crates/opentelemetry-etw-metrics)

This crate contains OpenTelemetry metrics exporter to
[ETW (Event Tracing for Windows)](https://learn.microsoft.com/en-us/windows-hardware/drivers/devtest/event-tracing-for-windows--etw-), a Windows solution
for efficient tracing of kernel or application-defined events, similar to user-events on Linux.
ETW events can be consumed in real-time or from a log file.

ETW events created with this crate can be generated and collected on Windows Vista or later.

This ETW exporter enables applications to use OpenTelemetry APIs to capture telemetry events and write to the ETW subsystem. From ETWs, the events can be
captured by agents running locally and listening for specific ETW events.

## Viewing ETW Logs
ETW logs can be viewed numerous ways, including through the use of (traceview)[https://learn.microsoft.com/en-us/windows-hardware/drivers/devtest/traceview] and (tracelog)[https://learn.microsoft.com/en-us/windows-hardware/drivers/devtest/tracelog] + (tracefmt)[https://learn.microsoft.com/en-us/windows-hardware/drivers/devtest/tracefmt]. `Traceview` provides a GUI while `tracelog` is geared towards command line usage.

### Traceview
After downloading `traceview`, its GUI can be spawned by invoking `traceview.exe`. A new session can be created via `File -> Create New Log Session` and then specifying a method for obtaining the control GUID. This crate currently writes traces with a GUID of `EDC24920-E004-40F6-A8E1-0E6E48F39D84` so choosing the `Manually Entered Control GUID or Hashed Name` option and entering `EDC24920-E004-40F6-A8E1-0E6E48F39D84` will correctly capture events.

### Tracelog
As a command line utility, a `tracelog` session can be run using the following commands:
- `tracelog -start MyTraceSession -f MyTraceFile.etl -guid #EDC24920-E004-40F6-A8E1-0E6E48F39D84`
- <run Rust code to emit ETW events>
- `tracelog -stop MyTraceSession`
- `tracefmt -o MyTraceFile.txt MyTraceFile.etl`
- `notepad MyTraceFile.txt`