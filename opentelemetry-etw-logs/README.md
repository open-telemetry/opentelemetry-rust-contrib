# OpenTelemetry Log Exporter for ETW

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

This crate contains OpenTelemetry Log exporter to
[ETW (Event Tracing for Windows)](https://learn.microsoft.com/en-us/windows-hardware/drivers/devtest/event-tracing-for-windows--etw-), a Windows solution
for efficient tracing of kernel or application-defined events, similar to user_events on Linux.
ETW events can be consumed in real-time or from a log file.

ETW events created with this crate can be generated and collected on Windows Vista or later.

This ETW exporter enables applications to use OpenTelemetry APIs to capture telemetry events and write to the ETW subsystem. From ETWs, the events can be
captured by agents running locally and listening for specific ETW events.

[![Crates.io: opentelemetry-etw-logs](https://img.shields.io/crates/v/opentelemetry-etw-logs.svg)](https://crates.io/crates/opentelemetry-etw-logs)
[![Documentation](https://docs.rs/opentelemetry-etw-logs/badge.svg)](https://docs.rs/opentelemetry-etw-logs)
[![LICENSE](https://img.shields.io/crates/l/opentelemetry-etw-logs)](./LICENSE)
[![GitHub Actions CI](https://github.com/open-telemetry/opentelemetry-rust/workflows/CI/badge.svg)](https://github.com/open-telemetry/opentelemetry-rust/actions?query=workflow%3ACI+branch%3Amain)
[![Slack](https://img.shields.io/badge/slack-@cncf/otel/rust-brightgreen.svg?logo=slack)](https://cloud-native.slack.com/archives/C03GDP0H023)

## Viewing ETW Logs
Logs exported to ETW can be viewed using tools like `logman`, `perfview` etc.

#### Instructions using Powershell:
1. Get the ETW Session Guid for the given provider (on this example `my-provider-name`):<br />
   ``PS> $EtwSessionGuid = (new-object System.Diagnostics.Tracing.EventSource("my-provider-name")).Guid.ToString()``
2. Start Logman session:<br />
   ```PS> logman create trace OtelETWExampleBasic -o OtelETWExampleBasic.log -p "{$EtwSessionGuid}" -f bincirc -max 1000```
   ```PS> logman start OtelETWExampleBasic```
3. Stop and Remove `logman` session:<br />
   ```PS> logman stop OtelETWExampleBasic```<br />
   ```PS> logman delete OtelETWExampleBasic```<br />
4. View the events with `perfview`:<br />
   4.1. [Download PerView](https://github.com/microsoft/perfview/blob/main/documentation/Downloading.md):[PerfView releases](https://github.com/Microsoft/perfview/releases)
   4.2. Open PerfView.
   4.3. Go the location of the `.etl` file: `OtelETWExampleBasic.log_000001.etl` and open it.
   4.4. Double-click `Events` in the left-panel.
   4.5. Double-click the `my-provider-name/my-event-name` in the left-panel.
   4.6. You should see the events in the right-panel.
   
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
