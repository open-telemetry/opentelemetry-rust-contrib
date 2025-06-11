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

use opentelemetry::logs::AnyValue;
use opentelemetry::logs::LogRecord;
use opentelemetry::logs::Logger;
use opentelemetry::logs::LoggerProvider;
use opentelemetry::logs::Severity;
use opentelemetry::{Key, SpanId, TraceId};
use opentelemetry_sdk::logs::SdkLoggerProvider;
use std::collections::HashMap;
use std::time::SystemTime;

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
    let logger = logger_provider.logger("ContosoLogger2");

    let enabled = logger.event_enabled(Severity::Error, "otel-target", Some("event-name-otel"));

    println!("Logger is enabled: {}, for event-name-otel", enabled);

    for _ in 0..100 {
        // no trace context
        // without trace context
        let boolean = true;
        let int = 42;
        let float = 3.14;
        let string = "string";
        let bytes = b"bytes";
        let list_boolean = vec![AnyValue::Boolean(true), AnyValue::Boolean(false)];
        let list_int = vec![AnyValue::Int(42), AnyValue::Int(64)];
        let list_float = vec![AnyValue::Double(3.14), AnyValue::Double(2.71)];
        let list_string = vec![
            AnyValue::String("string".into()),
            AnyValue::String("string2".into()),
        ];
        let list_bytes = vec![
            AnyValue::Bytes(Box::new(b"bytes1".to_vec())),
            AnyValue::Bytes(Box::new(b"bytes2".to_vec())),
        ];
        let list_list_any = vec![
            AnyValue::ListAny(Box::new(vec![AnyValue::Int(18), AnyValue::Int(30)])),
            AnyValue::ListAny(Box::new(vec![AnyValue::Int(42), AnyValue::Int(64)])),
        ];
        let list_map_any = vec![
            AnyValue::Map(Box::new(HashMap::from([(
                Key::from_static_str("key"),
                AnyValue::String("value".into()),
            )]))),
            AnyValue::Map(Box::new(HashMap::from([(
                Key::from_static_str("key2"),
                AnyValue::String("value2".into()),
            )]))),
        ];
        let map_boolean = HashMap::from([
            (Key::from_static_str("key"), AnyValue::Boolean(true)),
            (Key::from_static_str("key2"), AnyValue::Boolean(false)),
        ]);
        let map_int = HashMap::from([
            (Key::from_static_str("key"), AnyValue::Int(42)),
            (Key::from_static_str("key2"), AnyValue::Int(64)),
        ]);
        let map_float = HashMap::from([
            (Key::from_static_str("key"), AnyValue::Double(3.14)),
            (Key::from_static_str("key2"), AnyValue::Double(2.71)),
        ]);
        let map_string = HashMap::from([
            (
                Key::from_static_str("key"),
                AnyValue::String("string".into()),
            ),
            (
                Key::from_static_str("key2"),
                AnyValue::String("string2".into()),
            ),
        ]);
        let map_bytes = HashMap::from([
            (
                Key::from_static_str("key"),
                AnyValue::Bytes(Box::new(b"bytes1".to_vec())),
            ),
            (
                Key::from_static_str("key2"),
                AnyValue::Bytes(Box::new(b"bytes2".to_vec())),
            ),
        ]);
        let map_list_any = HashMap::from([
            (
                Key::from_static_str("key"),
                AnyValue::ListAny(Box::new(vec![AnyValue::Int(18), AnyValue::Int(30)])),
            ),
            (
                Key::from_static_str("key2"),
                AnyValue::ListAny(Box::new(vec![AnyValue::Int(42), AnyValue::Int(64)])),
            ),
        ]);
        let map_map_any = HashMap::from([
            (
                Key::from_static_str("key"),
                AnyValue::Map(Box::new(HashMap::from([(
                    Key::from_static_str("key"),
                    AnyValue::String("value".into()),
                )]))),
            ),
            (
                Key::from_static_str("key2"),
                AnyValue::Map(Box::new(HashMap::from([(
                    Key::from_static_str("key2"),
                    AnyValue::String("value2".into()),
                )]))),
            ),
        ]);

        let mut record = logger.create_log_record();

        record.set_event_name("event-name-otel");
        record.set_severity_number(Severity::Error);
        record.set_severity_text("Error");
        record.set_body("This is an otel message".into());
        record.set_timestamp(SystemTime::now());
        record.set_target("otel-target");
        record.set_trace_context(TraceId::from(42), SpanId::from(42), None);

        let mut attributes = Vec::<(Key, AnyValue)>::new();
        attributes.push(("boolean".into(), boolean.into()));
        attributes.push(("int".into(), int.into()));
        attributes.push(("float".into(), float.into()));
        attributes.push(("string".into(), string.into()));
        attributes.push(("bytes".into(), AnyValue::Bytes(Box::new(bytes.to_vec()))));
        attributes.push((
            "list_boolean".into(),
            AnyValue::ListAny(Box::new(list_boolean)),
        ));
        attributes.push(("list_int".into(), AnyValue::ListAny(Box::new(list_int))));
        attributes.push(("list_float".into(), AnyValue::ListAny(Box::new(list_float))));
        attributes.push((
            "list_string".into(),
            AnyValue::ListAny(Box::new(list_string)),
        ));
        attributes.push(("list_bytes".into(), AnyValue::ListAny(Box::new(list_bytes))));
        attributes.push((
            "list_list_any".into(),
            AnyValue::ListAny(Box::new(list_list_any)),
        ));
        attributes.push((
            "list_map_any".into(),
            AnyValue::ListAny(Box::new(list_map_any)),
        ));
        attributes.push(("map_boolean".into(), AnyValue::Map(Box::new(map_boolean))));
        attributes.push(("map_int".into(), AnyValue::Map(Box::new(map_int))));
        attributes.push(("map_float".into(), AnyValue::Map(Box::new(map_float))));
        attributes.push(("map_string".into(), AnyValue::Map(Box::new(map_string))));
        attributes.push(("map_bytes".into(), AnyValue::Map(Box::new(map_bytes))));
        attributes.push(("map_list_any".into(), AnyValue::Map(Box::new(map_list_any))));
        attributes.push(("map_map_any".into(), AnyValue::Map(Box::new(map_map_any))));
        record.add_attributes(attributes);

        logger.emit(record);

        // tracing_test::test_tracing_error_inside_lib();
        // tracing_test::my_secret_module::test_tracing_error_inside_module();
    }
}
