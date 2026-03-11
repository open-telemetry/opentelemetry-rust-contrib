# OpenTelemetry Trace Exporter for ETW

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

| Status        |           |
| ------------- |-----------|
| Stability     | beta      |
| Owners        |           |

This crate contains an OpenTelemetry Trace exporter to
[ETW (Event Tracing for Windows)](https://learn.microsoft.com/en-us/windows-hardware/drivers/devtest/event-tracing-for-windows--etw-), a Windows solution
for efficient tracing of kernel or application-defined events, similar to user_events on Linux.
ETW events can be consumed in real-time or from a log file.

ETW events created with this crate can be generated and collected on Windows Vista or later.

This ETW exporter enables applications to use OpenTelemetry APIs to capture span data and write to the ETW subsystem. From ETW, the events can be
captured by agents running locally and listening for specific ETW events.

Spans are encoded following the [Microsoft Common Schema v4.0](https://learn.microsoft.com/en-us/opentelemetry/common-schema) format using
[TraceLogging Dynamic](https://crates.io/crates/tracelogging_dynamic).

## Viewing ETW Traces

Traces exported to ETW can be viewed using tools like `logman`, `perfview` etc.

### Using `logman`

To view the telemetry emitted to ETW you can use [`logman`](https://learn.microsoft.com/windows-server/administration/windows-commands/logman) along with `perfview`.
`logman` will listen to ETW events from the given provider (on this example, `provider_name`) and store them in a `.etl` file.

[`perfview`](https://github.com/microsoft/perfview) will allow you to visualize the events.

Instructions using Powershell:

1. Get the ETW Session Guid for the given provider (on this example `provider_name`):

    ```ps
    $EtwSessionGuid = (new-object System.Diagnostics.Tracing.EventSource("provider_name")).Guid.ToString()
    ```

1. Start Logman session:

    ```ps
    logman create trace OtelETWTracesBasic -o OtelETWTracesBasic.log -p "{$EtwSessionGuid}" -f bincirc -max 1000
    logman start OtelETWTracesBasic
    ```

1. Execute the example:

    ```ps
    cd opentelemetry-etw-traces
    cargo run --example basic
    ```

1. Stop and Remove `logman` session:

    ```ps
    logman stop OtelETWTracesBasic
    logman delete OtelETWTracesBasic
    ```

1. View the events with `perfview`:

    - Download PerfView: [Instructions](https://github.com/microsoft/perfview/blob/main/documentation/Downloading.md), [Releases](https://github.com/Microsoft/perfview/releases).
    - Open PerfView.
    - Go the location of the `.etl` file: `OtelETWTracesBasic.log_000001.etl` and open it.
    - Double-click `Events` in the left-panel.
    - Double-click the `provider_name/Span` in the left-panel.
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
