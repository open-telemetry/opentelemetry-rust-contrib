//! Basic example of logs instrumentation with the opentelemetry-etw-logs crate.

//! This example demonstrates how to use the opentelemetry-etw-logs crate with the tracing crate.
//!
//! run with `$ cargo run --example basic --all-features
//!
//! To view the telemetry emitted to ETW you can use [`logman`](https://learn.microsoft.com/windows-server/administration/windows-commands/logman) along with `perfview`. `logman` will listen to ETW
//! events from the given provider (on this example, `provider-name`) and store them in a `.etl` file.
//! [`perfview`](https://github.com/microsoft/perfview) will allow you to visualize the events.
//!
//! Instructions using Powershell:
//!
//! 1. Get the ETW Session Guid for the given provider (on this example `provider-name`):
//!   ```
//!   $EtwSessionGuid = (new-object System.Diagnostics.Tracing.EventSource("provider-name")).Guid.ToString()`
//!   ```
//! 1. Start Logman session:
//!   ```
//!   logman create trace OtelETWExampleBasic -o OtelETWExampleBasic.log -p "{$EtwSessionGuid}" -f bincirc -max 1000
//!   logman start OtelETWExampleBasic
//!   ```
//! 1. Execute this example:
//!    ```
//!    cd opentelemetry-etw-logs
//!    cargo run --example basic
//!    ```
//! 1. Stop and Remove `logman` session:
//!    ```
//!   logman stop OtelETWExampleBasic
//!   logman delete OtelETWExampleBasic
//!    ```
//! 1. View the events with `perfview`:
//!    a. [Download PerView](https://github.com/microsoft/perfview/blob/main/documentation/Downloading.md):[PerfView releases](https://github.com/Microsoft/perfview/releases).
//!    a. Open PerfView.
//!    a. Go the location of the `.etl` file: `OtelETWExampleBasic.log_000001.etl` and open it.
//!    a. Double-click `Events` in the left-panel.
//!    a. Double-click the `provider-name/event-name` in the left-panel.
//!    a. You should see the events in the right-panel.
//!

use opentelemetry_appender_tracing::layer;
use opentelemetry_etw_logs::ETWLoggerProviderBuilderExt;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use tracing::error;
use tracing_subscriber::prelude::*;

fn init_logger() -> SdkLoggerProvider {
    SdkLoggerProvider::builder()
        .with_etw_exporter("provider-name")
        .build()
}

fn main() {
    // Example with tracing appender.
    let logger_provider = init_logger();
    let layer = layer::OpenTelemetryTracingBridge::new(&logger_provider);
    tracing_subscriber::registry().with(layer).init();

    error!(
        name: "event-name",
        event_id = 20,
        user_name = "otel user",
        user_email = "otel@opentelemetry.io"
    );
}
