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

use log::{error, info, warn, Level};
use opentelemetry_appender_log::OpenTelemetryLogBridge;
use opentelemetry_etw_logs::ETWLoggerProviderBuilderExt;
use opentelemetry_etw_logs::ExporterOptions;
use opentelemetry_sdk::logs::SdkLoggerProvider;

use std::collections::HashMap;

fn init_logger() -> SdkLoggerProvider {
    let exporter_options = ExporterOptions::builder("ContosoProvider")
        .with_default_event_name("Log")
        .build()
        .unwrap();

    SdkLoggerProvider::builder()
        .with_etw_exporter(exporter_options)
        .build()
}

fn main() {
    // Example with tracing appender.
    let logger_provider = init_logger();
    let otel_log_appender = OpenTelemetryLogBridge::new(&logger_provider);
    log::set_boxed_logger(Box::new(otel_log_appender)).unwrap();
    log::set_max_level(Level::Info.to_level_filter());

    for _ in 0..100 {
        // error!(target: "my_target", fruit, price; "hello from {fruit}. My price is {price}");
        // error!(fruit, price; "hello from {fruit}. My price is {price}");

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

        error!(boolean, int, float, string, bytes; "This is log body");
    }

    let _ = logger_provider.shutdown();
}
