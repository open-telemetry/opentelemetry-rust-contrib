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
use opentelemetry_sdk::logs::SdkLoggerProvider;
use std::collections::HashMap;
use tracing::{error, span};
use tracing_subscriber::prelude::*;

fn init_logger() -> SdkLoggerProvider {
    let processor = opentelemetry_etw_logs::Processor::builder("ContosoProvider")
        .build()
        .expect("Valid provider name is required to build an ETW Processor.");

    SdkLoggerProvider::builder()
        .with_log_processor(processor)
        .build()
}

fn main() {
    // Example with tracing appender.
    let logger_provider = init_logger();
    let layer = layer::OpenTelemetryTracingBridge::new(&logger_provider);
    tracing_subscriber::registry().with(layer).init();

    let span = span!(tracing::Level::TRACE, "my_span");
    let _enter = span.enter();

    for _ in 0..100 {
        // no trace context
        // without trace context
        let boolean = true;
        let int = 42;
        let float = 3.14;
        let string = "string";
        let bytes = b"bytes";
        let list_boolean = vec![true, false];
        let list_int = vec![42, 64];
        let list_float = vec![3.14, 2.71];
        let list_string = vec!["string", "string2"];
        let list_bytes = vec![b"bytes1", b"bytes2"];
        let list_list_any = vec![vec![18, 30], vec![42, 64]];
        let list_map_any = vec![
            HashMap::from([("key", "value")]),
            HashMap::from([("key2", "value2")]),
        ];
        let map_boolean = HashMap::from([("key", true), ("key2", false)]);
        let map_int = HashMap::from([("key", 42), ("key2", 64)]);
        let map_float = HashMap::from([("key", 3.14), ("key2", 2.71)]);
        let map_string = HashMap::from([("key", "string"), ("key2", "string2")]);
        let map_bytes = HashMap::from([("key", b"bytes1"), ("key2", b"bytes2")]);
        let map_list_any = HashMap::from([("key", vec![18, 30]), ("key2", vec![42, 64])]);
        let map_map_any = HashMap::from([
            ("key", HashMap::from([("key", "value")])),
            ("key2", HashMap::from([("key2", "value2")])),
        ]);

        error!(name: "event-name-with-span", boolean, int, float, string, "This is tracing body");

        // error!(
        //   name: "event-name",
        //   event_id = 20,
        //   user_name = "otel user",
        //   user_email = "otel@opentelemetry.io",
        //   "main event"
        // );

        // tracing_test::test_tracing_error_inside_lib();
        // tracing_test::my_secret_module::test_tracing_error_inside_module();
    }
}
