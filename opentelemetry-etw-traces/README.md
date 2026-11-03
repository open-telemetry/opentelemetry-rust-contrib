# OpenTelemetry Trace Exporter for ETW

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

| Status        |           |
| ------------- |-----------|
| Stability     | beta      |
| Owners        | [Jelena Vidakovic](https://github.com/jevidako), [Uros Jovanovic](https://github.com/urosjovanovic) |

This crate contains an OpenTelemetry Trace exporter to
[ETW (Event Tracing for Windows)](https://learn.microsoft.com/en-us/windows-hardware/drivers/devtest/event-tracing-for-windows--etw-), a Windows solution
for efficient tracing of kernel or application-defined events, similar to user_events on Linux.
ETW events can be consumed in real-time or from a log file.

ETW events created with this crate can be generated and collected on Windows Vista or later.

This ETW exporter enables applications to use OpenTelemetry APIs to capture span data and write to the ETW subsystem. From ETW, the events can be
captured by agents running locally and listening for specific ETW events.

Spans are encoded following the [Microsoft Common Schema v4.0](https://learn.microsoft.com/en-us/opentelemetry/common-schema) format using
[TraceLogging Dynamic](https://crates.io/crates/tracelogging_dynamic).

[![Crates.io: opentelemetry-etw-traces](https://img.shields.io/crates/v/opentelemetry-etw-traces.svg)](https://crates.io/crates/opentelemetry-etw-traces)
[![Documentation](https://docs.rs/opentelemetry-etw-traces/badge.svg)](https://docs.rs/opentelemetry-etw-traces)
[![Slack](https://img.shields.io/badge/slack-@cncf/otel/rust-brightgreen.svg?logo=slack)](https://cloud-native.slack.com/archives/C03GDP0H023)

## Optional Geneva-compatible output

The default output uses `__csver__` and nested `PartA`, `PartB`, and `PartC`
structures. For collectors that do not translate that representation into
Geneva DGrep field names, enable the mapping when initializing the processor:

```rust
use opentelemetry_etw_traces::Processor;

let processor = Processor::builder("MyAppTracing")
    .with_geneva_mapping(true)
    .build()
    .expect("Failed to create ETW processor");
```

This mode emits flat `env_ver` (`"4.0"`), `env_name` (the ETW event name),
`env_time` (the span end time), `env_dt_traceId`, and `env_dt_spanId` fields.
Nonzero trace flags are emitted as `env_dt_traceFlags`; service resource
attributes become `env_cloud_role` and `env_cloud_roleInstance` when present.
Part B/C fields keep their existing names, types, and values without the
wrapper structures. `__csver__` and `_typeName` are not emitted in this mode.
Collection and routing still need to be configured in the agent.

Attribute names are not validated in this mode, to avoid per-attribute
overhead. Because every field is written at the top level, a span or promoted
resource attribute with the same name as an emitted field (`env_*`, `name`,
`kind`, `startTime`, `parentId`, `links`, `statusMessage`, `success`, or a
mapped Part B name such as `httpMethod`) produces duplicate field names; how
duplicates are handled depends on the collector. Collectors that replace
characters such as `.` with `_` can also create duplicates (for example,
`env.time` and `env_time`). Avoid these names for custom attributes.

Omitting `.with_geneva_mapping(...)`, or passing `false`, retains the current
output. Use a consistent mode for processors sharing a provider/event name;
enabling this option changes the event's schema.

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
