//! Basic example of traces instrumentation with the opentelemetry-etw-traces crate.

//! This example demonstrates how to use the opentelemetry-etw-traces crate with
//! the OpenTelemetry SDK tracing API.
//!
//! Run with `$ cargo run --example basic`
//!
//! To view the telemetry emitted to ETW you can use [`logman`](https://learn.microsoft.com/windows-server/administration/windows-commands/logman) along with `perfview`. `logman` will listen to ETW
//! events from the given provider (on this example, `provider_name`) and store them in a `.etl` file.
//! [`perfview`](https://github.com/microsoft/perfview) will allow you to visualize the events.
//!
//! Instructions using Powershell:
//!
//! 1. Get the ETW Session Guid for the given provider (on this example `provider_name`):
//!   ```
//!   $EtwSessionGuid = (new-object System.Diagnostics.Tracing.EventSource("provider_name")).Guid.ToString()
//!   ```
//! 1. Start Logman session:
//!   ```
//!   logman create trace OtelETWTracesBasic -o OtelETWTracesBasic.log -p "{$EtwSessionGuid}" -f bincirc -max 1000
//!   logman start OtelETWTracesBasic
//!   ```
//! 1. Execute this example:
//!    ```
//!    cd opentelemetry-etw-traces
//!    cargo run --example basic
//!    ```
//! 1. Stop and Remove `logman` session:
//!    ```
//!   logman stop OtelETWTracesBasic
//!   logman delete OtelETWTracesBasic
//!    ```
//! 1. View the events with `perfview`:
//!    a. [Download PerfView](https://github.com/microsoft/perfview/blob/main/documentation/Downloading.md): [PerfView releases](https://github.com/Microsoft/perfview/releases).
//!    a. Open PerfView.
//!    a. Go the location of the `.etl` file: `OtelETWTracesBasic.log_000001.etl` and open it.
//!    a. Double-click `Events` in the left-panel.
//!    a. Double-click the `provider_name/Span` in the left-panel.
//!    a. You should see the events in the right-panel.
//!

use opentelemetry::trace::Tracer;
use opentelemetry::trace::TracerProvider;
use opentelemetry_etw_traces::Processor;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;

fn init_tracer_provider() -> SdkTracerProvider {
    let processor = Processor::builder("provider_name")
        .build()
        .expect("Valid provider name is required to build an ETW Processor.");

    SdkTracerProvider::builder()
        .with_span_processor(processor)
        .with_resource(
            Resource::builder()
                .with_service_name("basic-example")
                .build(),
        )
        .build()
}

fn main() {
    let tracer_provider = init_tracer_provider();
    let tracer = tracer_provider.tracer("example-tracer");

    tracer.in_span("parent-operation", |_cx| {
        tracer.in_span("child-operation", |_cx| {});
    });

    tracer_provider
        .shutdown()
        .expect("TracerProvider should shutdown successfully.");
}
