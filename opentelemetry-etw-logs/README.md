# OpenTelemetry Log Exporter for ETW

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

| Status        |           |
| ------------- |-----------|
| Stability     | beta      |
| Owners        | [Cijo Thomas](https://github.com/cijothomas), [Lalit Kumar Bhasin](https://github.com/lalitb) |

This crate contains OpenTelemetry Log exporter to
[ETW (Event Tracing for Windows)](https://learn.microsoft.com/en-us/windows-hardware/drivers/devtest/event-tracing-for-windows--etw-), a Windows solution
for efficient tracing of kernel or application-defined events, similar to user_events on Linux.
ETW events can be consumed in real-time or from a log file.

ETW events created with this crate can be generated and collected on Windows Vista or later.

This ETW exporter enables applications to use OpenTelemetry APIs to capture telemetry events and write to the ETW subsystem. From ETW, the events can be
captured by agents running locally and listening for specific ETW events.

[![Crates.io: opentelemetry-etw-logs](https://img.shields.io/crates/v/opentelemetry-etw-logs.svg)](https://crates.io/crates/opentelemetry-etw-logs)
[![Documentation](https://docs.rs/opentelemetry-etw-logs/badge.svg)](https://docs.rs/opentelemetry-etw-logs)
[![LICENSE](https://img.shields.io/crates/l/opentelemetry-etw-logs)](./LICENSE)
[![GitHub Actions CI](https://github.com/open-telemetry/opentelemetry-rust-contrib/workflows/CI/badge.svg)](https://github.com/open-telemetry/opentelemetry-rust-contrib/actions?query=workflow%3ACI+branch%3Amain)
[![Slack](https://img.shields.io/badge/slack-@cncf/otel/rust-brightgreen.svg?logo=slack)](https://cloud-native.slack.com/archives/C03GDP0H023)

## Viewing ETW Logs

Logs exported to ETW can be viewed using tools like `logman`, `perfview` etc.

### Using `logman`

To view the telemetry emitted to ETW you can use [`logman`](https://learn.microsoft.com/windows-server/administration/windows-commands/logman) along with `perfview`.
`logman` will listen to ETW events from the given provider (on this example, `provider-name`) and store them in a `.etl` file.

[`perfview`](https://github.com/microsoft/perfview) will allow you to visualize the events.

Instructions using Powershell:

1. Get the ETW Session Guid for the given provider (on this example `provider-name`):

    ```ps
    $EtwSessionGuid = (new-object System.Diagnostics.Tracing.EventSource("provider-name")).Guid.ToString()
    ```

1. Start Logman session:

    ```ps
    logman create trace OtelETWExampleBasic -o OtelETWExampleBasic.log -p "{$EtwSessionGuid}" -f bincirc -max 1000
    logman start OtelETWExampleBasic
    ```

1. Execute this example:

    ```ps
    cd opentelemetry-etw-logs
    cargo run --example basic
    ```

1. Stop and Remove `logman` session:

    ```ps
    logman stop OtelETWExampleBasic
    logman delete OtelETWExampleBasic
    ```

1. View the events with `perfview`:

    - Download PerfView: [Instructions](https://github.com/microsoft/perfview/blob/main/documentation/Downloading.md), [Releases](https://github.com/Microsoft/perfview/releases).
    - Open PerfView.
    - Go the location of the `.etl` file: `OtelETWExampleBasic.log_000001.etl` and open it.
    - Double-click `Events` in the left-panel.
    - Double-click the `provider-name/event-name` in the left-panel.
    - You should see the events in the right-panel.

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
