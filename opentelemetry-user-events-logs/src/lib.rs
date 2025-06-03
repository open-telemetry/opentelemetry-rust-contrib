//! # OpenTelemetry User Events Exporter for Logs
//!
//! This crate provides a log exporter for exporting logs to the Linux
//! [user_events](https://docs.kernel.org/trace/user_events.html) subsystem. The
//! `user_events` subsystem is a Linux kernel feature introduced in version 6.4,
//! designed for efficient user process tracing. It is conceptually similar to
//! Event Tracing for Windows (ETW) on Windows and leverages Linux Tracepoints
//! to enable user processes to create traceable events and data. These events
//! can be analyzed using existing tools like `ftrace` and `perf`.
//!
//! ## Key Features of `user_events`
//!
//! - **Efficient Tracing Path**: Provides a faster path for tracing from
//!   user-mode applications by utilizing kernel-mode memory address space.
//! - **Selective Event Export**: Allows user processes to export telemetry
//!   events only when they are actively needed, i.e., when the corresponding
//!   tracepoint events are enabled.
//!
//! ## Purpose of this Exporter
//!
//! The `user_events` exporter enables applications to use the OpenTelemetry API
//! to capture telemetry events and write them to the `user_events` subsystem.
//! Once written, these events can be:
//!
//! - **Captured by Local Agents**: Agents running locally can listen for
//!   specific events within the `user_events` subsystem.
//! - **Monitored in Real-Time**: Events can be monitored in real-time using
//!   Linux tools like `perf` or `ftrace`.
//!
//! ## Prerequisite
//!
//! - **Linux Kernel Version**: Requires Linux kernel 6.4 or later with
//!   `user_events` support enabled to use the exporter.
//!
//! ## Synchronous Export
//!
//! This exporter writes telemetry events to the `user_events` subsystem
//! synchronously, without any buffering or batching. The exporter is
//! non-blocking, and each event is immediately exported, ensuring that no
//! telemetry is lost in the event the application crashes.
//!
//! ## Example Use Case
//!
//! Applications can use this exporter to:
//!
//! - Emit logs to the `user_events` subsystem.
//! - Enable local agents or monitoring tools to capture and analyze these
//!   events for debugging or performance monitoring.
//!
//! For more details on the `user_events` subsystem, refer to the [official
//! documentation](https://docs.kernel.org/trace/user_events.html).
//!
//! ## Getting Started
//!
//! To use the `user_events` exporter, you can set up a logger provider as follows:
//!
//! ```rust
//! use opentelemetry_sdk::logs::SdkLoggerProvider;
//! use opentelemetry_sdk::Resource;
//! use opentelemetry_user_events_logs::Processor;
//!
//! let user_event_processor = Processor::builder("myprovider")
//!   .build()
//!   .unwrap_or_else(|err| {
//!     eprintln!("Failed to create user_events processor. Error: {}", err);
//!     panic!("exiting due to error during initialization");
//!              });
//!
//! let provider = SdkLoggerProvider::builder()
//!     .with_resource(
//!         Resource::builder_empty()
//!             .with_service_name("example")
//!             .build(),
//!     )
//!     .with_log_processor(user_event_processor)
//!     .build();
//! ```
//!
//! This will create a logger provider with the `user_events` exporter enabled.
//!
//! ## Listening to Exported Events
//!
//! Tools like `perf` or `ftrace` can be used to listen to the exported events.
//!
//! - **Using `perf`**: For instance, the following command can be used to
//!   record events of severity `Error` and `Warning`:
//!   ```bash
//!   perf record -e user_events:myprovider_L2K1,user_events:myprovider_L3K1
//!   ```

#![warn(missing_debug_implementations, missing_docs)]

mod logs;

pub use logs::Processor;
pub use logs::ProcessorBuilder;

#[cfg(test)]
mod tests {

    use crate::Processor;
    use opentelemetry::trace::Tracer;
    use opentelemetry::trace::{TraceContextExt, TracerProvider};
    use opentelemetry::Key;
    use opentelemetry_appender_tracing::layer;
    use opentelemetry_sdk::Resource;
    use opentelemetry_sdk::{
        logs::LoggerProviderBuilder,
        trace::{Sampler, SdkTracerProvider},
    };
    use serde_json::{from_str, Value};
    use std::process::Command;
    use tracing::error;
    use tracing_subscriber::{layer::SubscriberExt, EnvFilter, Layer};

    // Ignore as this cannot be run in Github CI due to lack of
    // required Kernel. Uncomment to run locally in a supported environment

    #[ignore]
    #[test]
    fn integration_test_basic() {
        // Run using the below command
        // sudo -E ~/.cargo/bin/cargo test integration_test_basic -- --nocapture --ignored

        // Basic check if user_events are available
        check_user_events_available().expect("Kernel does not support user_events. Verify your distribution/kernel supports user_events: https://docs.kernel.org/trace/user_events.html.");
        let user_event_processor = Processor::builder("myprovider").build().unwrap();

        let logger_provider = LoggerProviderBuilder::default()
            .with_resource(Resource::builder().with_service_name("myrolename").build())
            .with_log_processor(user_event_processor)
            .build();

        // Once provider with user_event exporter is created, it should create the TracePoints
        // following providername_level_k1 format
        // Validate that the TracePoints are created.
        let user_event_status = check_user_events_available().expect("Kernel does not support user_events. Verify your distribution/kernel supports user_events: https://docs.kernel.org/trace/user_events.html.");
        assert!(user_event_status.contains("myprovider_L1K1"));
        assert!(user_event_status.contains("myprovider_L2K1"));
        assert!(user_event_status.contains("myprovider_L3K1"));
        assert!(user_event_status.contains("myprovider_L4K1"));
        assert!(user_event_status.contains("myprovider_L5K1"));

        let filter_otel =
            EnvFilter::new("info").add_directive("opentelemetry=off".parse().unwrap());
        let otel_layer = layer::OpenTelemetryTracingBridge::new(&logger_provider);
        let otel_layer = otel_layer.with_filter(filter_otel);
        let subscriber = tracing_subscriber::registry().with(otel_layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        // Start perf recording in a separate thread and emit logs in parallel.
        let perf_thread =
            std::thread::spawn(|| run_perf_and_decode(5, "user_events:myprovider_L2K1"));

        // Give a little time for perf to start recording
        std::thread::sleep(std::time::Duration::from_millis(1000));

        // ACT
        error!(
            name: "my-event-name",
            target: "my-target",
            event_id = 20,
            bool_field = true,
            double_field = 1.0,
            user_name = "otel user",
            user_email = "otel.user@opentelemetry.com",
            message = "This is a test message",
        );

        // Wait for the perf thread to complete and get the results
        let result = perf_thread.join().expect("Perf thread panicked");

        assert!(result.is_ok());
        let json_content = result.unwrap();
        assert!(!json_content.is_empty());

        let formatted_output = json_content.trim().to_string();
        /*
                // Sample output from perf-decode
                {
        "./perf.data": [
          { "n": "myprovider:Log", "__csver__": 1024, "PartA": { "time": "2025-03-07T16:31:28.279214367+00:00", "ext_cloud_role": "myrolename"  }, "PartC": { "user_name": "otel user", "user_email": "otel.user@opentelemetry.com" }, "PartB": { "_typeName": "Log", "severityNumber": 2, "severityText": "ERROR", "eventId": 20, "name": "my-event-name" }, "meta": { "time": 81252.403220286, "cpu": 4, "pid": 21084, "tid": 21085, "level": 2, "keyword": "0x1" } } ]
        }
                 */

        let json_value: Value = from_str(&formatted_output).expect("Failed to parse JSON");
        let perf_data_key = json_value
            .as_object()
            .expect("JSON is not an object")
            .keys()
            .find(|k| k.contains("perf.data"))
            .expect("No perf.data key found in JSON");

        let events = json_value[perf_data_key]
            .as_array()
            .expect("Events for perf.data is not an array");

        // Find the specific event. Its named providername:eventname format.
        let event = events
            .iter()
            .find(|e| {
                if let Some(name) = e.get("n") {
                    name.as_str().unwrap_or("") == "myprovider:Log"
                } else {
                    false
                }
            })
            .expect("Event 'myprovider:Log' not found");

        // Validate event structure and fields
        assert_eq!(event["n"].as_str().unwrap(), "myprovider:Log");
        assert_eq!(event["__csver__"].as_i64().unwrap(), 1024);

        // Validate PartA
        let part_a = &event["PartA"];
        // Only check if the time field exists, not the actual value
        assert!(part_a.get("time").is_some(), "PartA.time is missing");

        let role = part_a
            .get("ext_cloud_role")
            .expect("PartA.ext_cloud_role is missing");
        assert_eq!(role.as_str().unwrap(), "myrolename");

        // Validate PartB
        let part_b = &event["PartB"];
        assert_eq!(part_b["_typeName"].as_str().unwrap(), "Log");
        assert_eq!(part_b["severityNumber"].as_i64().unwrap(), 17);
        assert_eq!(part_b["severityText"].as_str().unwrap(), "ERROR");
        assert_eq!(part_b["eventId"].as_i64().unwrap(), 20);
        assert_eq!(part_b["name"].as_str().unwrap(), "my-event-name");
        assert_eq!(part_b["body"].as_str().unwrap(), "This is a test message");

        // Validate PartC
        let part_c = &event["PartC"];
        assert_eq!(part_c["user_name"].as_str().unwrap(), "otel user");
        assert_eq!(
            part_c["user_email"].as_str().unwrap(),
            "otel.user@opentelemetry.com"
        );
        assert!(part_c["bool_field"].as_bool().unwrap());
        assert_eq!(part_c["double_field"].as_f64().unwrap(), 1.0);
    }

    #[ignore]
    #[test]
    fn integration_test_with_tracing() {
        // Run using the below command
        // sudo -E ~/.cargo/bin/cargo test integration_test_with_tracing -- --nocapture --ignored

        // Basic check if user_events are available
        check_user_events_available().expect("Kernel does not support user_events. Verify your distribution/kernel supports user_events: https://docs.kernel.org/trace/user_events.html.");

        // setup tracing
        let tracer_provider = SdkTracerProvider::builder()
            .with_sampler(Sampler::AlwaysOn)
            .build();
        let tracer = tracer_provider.tracer("test-tracer");

        let user_event_processor = Processor::builder("myprovider").build().unwrap();
        let logger_provider = LoggerProviderBuilder::default()
            .with_log_processor(user_event_processor)
            .build();

        // Once provider with user_event exporter is created, it should create the TracePoints
        // following providername_level_k1 format
        // Validate that the TracePoints are created.
        let user_event_status = check_user_events_available().expect("Kernel does not support user_events. Verify your distribution/kernel supports user_events: https://docs.kernel.org/trace/user_events.html.");
        assert!(user_event_status.contains("myprovider_L1K1"));
        assert!(user_event_status.contains("myprovider_L2K1"));
        assert!(user_event_status.contains("myprovider_L3K1"));
        assert!(user_event_status.contains("myprovider_L4K1"));
        assert!(user_event_status.contains("myprovider_L5K1"));

        let filter_otel =
            EnvFilter::new("info").add_directive("opentelemetry=off".parse().unwrap());
        let otel_layer = layer::OpenTelemetryTracingBridge::new(&logger_provider);
        let otel_layer = otel_layer.with_filter(filter_otel);
        let subscriber = tracing_subscriber::registry().with(otel_layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        // Start perf recording in a separate thread and emit logs in parallel.
        let perf_thread =
            std::thread::spawn(|| run_perf_and_decode(5, "user_events:myprovider_L2K1"));

        // Give a little time for perf to start recording
        std::thread::sleep(std::time::Duration::from_millis(1000));

        // ACT
        let (trace_id_expected, span_id_expected) = tracer.in_span("test-span", |cx| {
            let trace_id = cx.span().span_context().trace_id();
            let span_id = cx.span().span_context().span_id();

            // logging is done inside span context.
            error!(
                name: "my-event-name",
                target: "my-target",
                event_id = 20,
                user_name = "otel user",
                user_email = "otel.user@opentelemetry.com"
            );
            (trace_id, span_id)
        });

        // Wait for the perf thread to complete and get the results
        let result = perf_thread.join().expect("Perf thread panicked");

        assert!(result.is_ok());
        let json_content = result.unwrap();
        assert!(!json_content.is_empty());

        let formatted_output = json_content.trim().to_string();
        /*
                // Sample output from perf-decode
                {
        "./perf.data": [
          { "n": "myprovider:my-event-name", "__csver__": 1024, "PartA": { "time": "2025-03-07T16:31:28.279214367+00:00" }, "PartC": { "user_name": "otel user", "user_email": "otel.user@opentelemetry.com" }, "PartB": { "_typeName": "Log", "severityNumber": 2, "severityText": "ERROR", "eventId": 20, "name": "my-event-name" }, "meta": { "time": 81252.403220286, "cpu": 4, "pid": 21084, "tid": 21085, "level": 2, "keyword": "0x1" } } ]
        }
                 */

        let json_value: Value = from_str(&formatted_output).expect("Failed to parse JSON");
        let perf_data_key = json_value
            .as_object()
            .expect("JSON is not an object")
            .keys()
            .find(|k| k.contains("perf.data"))
            .expect("No perf.data key found in JSON");

        let events = json_value[perf_data_key]
            .as_array()
            .expect("Events for perf.data is not an array");

        // Find the specific event. Its named providername:eventname format.
        let event = events
            .iter()
            .find(|e| {
                if let Some(name) = e.get("n") {
                    name.as_str().unwrap_or("") == "myprovider:Log"
                } else {
                    false
                }
            })
            .expect("Event 'myprovider:Log' not found");

        // Validate event structure and fields
        assert_eq!(event["n"].as_str().unwrap(), "myprovider:Log");
        assert_eq!(event["__csver__"].as_i64().unwrap(), 1024);

        // Validate PartA
        let part_a = &event["PartA"];
        // Only check if the time field exists, not the actual value
        assert!(part_a.get("time").is_some(), "PartA.time is missing");

        let part_a_ext_dt_trace_id = part_a
            .get("ext_dt_traceId")
            .expect("PartA.ext_dt_traceId is missing");
        let part_a_ext_dt_span_id = part_a
            .get("ext_dt_spanId")
            .expect("PartA.ext_dt_spanId is missing");

        // Validate trace_id and span_id
        assert_eq!(
            part_a_ext_dt_trace_id.as_str().unwrap(),
            format!("{:x}", trace_id_expected)
        );
        assert_eq!(
            part_a_ext_dt_span_id.as_str().unwrap(),
            format!("{:x}", span_id_expected)
        );

        // Validate PartB
        let part_b = &event["PartB"];
        assert_eq!(part_b["_typeName"].as_str().unwrap(), "Log");
        assert_eq!(part_b["severityNumber"].as_i64().unwrap(), 17);
        assert_eq!(part_b["severityText"].as_str().unwrap(), "ERROR");
        assert_eq!(part_b["eventId"].as_i64().unwrap(), 20);
        assert_eq!(part_b["name"].as_str().unwrap(), "my-event-name");

        // Validate PartC
        let part_c = &event["PartC"];
        assert_eq!(part_c["user_name"].as_str().unwrap(), "otel user");
        assert_eq!(
            part_c["user_email"].as_str().unwrap(),
            "otel.user@opentelemetry.com"
        );
    }

    // Helper function to test direct logging (i.e without tracing or log crate)
    // with different severity levels
    fn integration_test_direct_helper(severity: opentelemetry::logs::Severity, trace_point: &str) {
        use opentelemetry::logs::AnyValue;
        use opentelemetry::logs::LogRecord;
        use opentelemetry::logs::Logger;
        use opentelemetry::logs::LoggerProvider;

        // Basic check if user_events are available
        check_user_events_available().expect("Kernel does not support user_events. Verify your distribution/kernel supports user_events: https://docs.kernel.org/trace/user_events.html.");
        let user_event_processor = Processor::builder("myprovider").build().unwrap();

        let logger_provider = LoggerProviderBuilder::default()
            .with_resource(Resource::builder().with_service_name("myrolename").build())
            .with_log_processor(user_event_processor)
            .build();

        let logger = logger_provider.logger("test");

        let mut record = logger.create_log_record();
        record.set_severity_number(severity);
        record.set_event_name("my-event-name");
        record.set_target("my-target");
        record.set_body(AnyValue::from("This is a test message"));
        // Add attributes for each AnyValue variant
        // String variant
        record.add_attribute("string_attr", "string value");

        // Int variant
        record.add_attribute("int_attr", 42i64);

        // Double variant
        record.add_attribute("double_attr", 3.575);

        // Boolean variant
        record.add_attribute("bool_attr", true);

        // Bytes variant
        let bytes_data = vec![1, 2, 3, 4, 5];
        record.add_attribute("bytes_attr", AnyValue::Bytes(Box::new(bytes_data)));

        // ListAny variant
        let list_values = vec![AnyValue::Int(1), AnyValue::Int(2), AnyValue::Int(3)];
        record.add_attribute("list_attr", AnyValue::ListAny(Box::new(list_values)));

        // Map variant
        let mut map_values = std::collections::HashMap::new();
        map_values.insert(Key::new("key1"), AnyValue::String("value1".into()));
        map_values.insert(Key::new("key2"), AnyValue::Int(42));
        record.add_attribute("map_attr", AnyValue::Map(Box::new(map_values)));

        // Once provider with user_event exporter is created, it should create the TracePoints
        // following providername_level_k1 format
        // Validate that the TracePoints are created.
        let user_event_status = check_user_events_available().expect("Kernel does not support user_events. Verify your distribution/kernel supports user_events: https://docs.kernel.org/trace/user_events.html.");
        assert!(user_event_status.contains("myprovider_L1K1"));
        assert!(user_event_status.contains("myprovider_L2K1"));
        assert!(user_event_status.contains("myprovider_L3K1"));
        assert!(user_event_status.contains("myprovider_L4K1"));
        assert!(user_event_status.contains("myprovider_L5K1"));

        // Start perf recording in a separate thread and emit logs in parallel.
        let trace_point_clone = trace_point.to_string();
        let perf_thread =
            std::thread::spawn(move || run_perf_and_decode(5, trace_point_clone.as_ref()));

        // Give a little time for perf to start recording
        std::thread::sleep(std::time::Duration::from_millis(1000));

        // ACT
        logger.emit(record);

        // Wait for the perf thread to complete and get the results
        let result = perf_thread.join().expect("Perf thread panicked");

        assert!(result.is_ok());
        let json_content = result.unwrap();
        assert!(!json_content.is_empty());

        let formatted_output = json_content.trim().to_string();
        /*
                // Sample output from perf-decode
                {
        "./perf.data": [
          { "n": "myprovider:Log", "__csver__": 1024, "PartA": { "time": "2025-03-07T16:31:28.279214367+00:00", "ext_cloud_role": "myrolename"  }, "PartC": { "user_name": "otel user", "user_email": "otel.user@opentelemetry.com" }, "PartB": { "_typeName": "Log", "severityNumber": 2, "severityText": "ERROR", "eventId": 20, "name": "my-event-name" }, "meta": { "time": 81252.403220286, "cpu": 4, "pid": 21084, "tid": 21085, "level": 2, "keyword": "0x1" } } ]
        }
                 */

        let json_value: Value = from_str(&formatted_output).expect("Failed to parse JSON");
        let perf_data_key = json_value
            .as_object()
            .expect("JSON is not an object")
            .keys()
            .find(|k| k.contains("perf.data"))
            .expect("No perf.data key found in JSON");

        let events = json_value[perf_data_key]
            .as_array()
            .expect("Events for perf.data is not an array");

        // Find the specific event. Its named providername:eventname format.
        let event = events
            .iter()
            .find(|e| {
                if let Some(name) = e.get("n") {
                    name.as_str().unwrap_or("") == "myprovider:Log"
                } else {
                    false
                }
            })
            .expect("Event 'myprovider:Log' not found");

        // Validate event structure and fields
        assert_eq!(event["n"].as_str().unwrap(), "myprovider:Log");
        assert_eq!(event["__csver__"].as_i64().unwrap(), 1024);

        // Validate PartA
        let part_a = &event["PartA"];
        // Only check if the time field exists, not the actual value
        assert!(part_a.get("time").is_some(), "PartA.time is missing");

        let role = part_a
            .get("ext_cloud_role")
            .expect("PartA.ext_cloud_role is missing");
        assert_eq!(role.as_str().unwrap(), "myrolename");

        // Validate PartB
        let part_b = &event["PartB"];
        assert_eq!(part_b["_typeName"].as_str().unwrap(), "Log");
        assert_eq!(part_b["severityNumber"].as_i64().unwrap(), severity as i64);
        assert_eq!(part_b["name"].as_str().unwrap(), "my-event-name");
        assert_eq!(part_b["body"].as_str().unwrap(), "This is a test message");

        // Validate PartC
        let part_c = &event["PartC"];
        assert_eq!(part_c["string_attr"].as_str().unwrap(), "string value");
        assert_eq!(part_c["int_attr"].as_i64().unwrap(), 42i64);
        assert_eq!(part_c["double_attr"].as_f64().unwrap(), 3.575);
        assert!(part_c["bool_attr"].as_bool().unwrap());

        // These are not supported currently, and are serialize as empty strings
        // which is validated here.
        assert_eq!(part_c["bytes_attr"].as_str().unwrap(), "");
        assert_eq!(part_c["list_attr"].as_str().unwrap(), "");
        assert_eq!(part_c["map_attr"].as_str().unwrap(), "");
    }

    #[ignore]
    #[test]
    fn integration_test_direct() {
        use opentelemetry::logs::Severity;
        // Run using the below command
        // sudo -E ~/.cargo/bin/cargo test integration_test_direct -- --nocapture --ignored
        integration_test_direct_helper(Severity::Debug, "user_events:myprovider_L5K1");
        integration_test_direct_helper(Severity::Info, "user_events:myprovider_L4K1");
        integration_test_direct_helper(Severity::Warn, "user_events:myprovider_L3K1");
        integration_test_direct_helper(Severity::Error, "user_events:myprovider_L2K1");
        integration_test_direct_helper(Severity::Fatal, "user_events:myprovider_L1K1");
    }

    fn check_user_events_available() -> Result<String, String> {
        let output = Command::new("sudo")
            .arg("cat")
            .arg("/sys/kernel/tracing/user_events_status")
            .output()
            .map_err(|e| format!("Failed to execute command: {}", e))?;

        if output.status.success() {
            let status = String::from_utf8_lossy(&output.stdout);
            Ok(status.to_string())
        } else {
            Err(format!(
                "Command executed with failing error code: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }

    pub fn run_perf_and_decode(duration_secs: u64, event: &str) -> std::io::Result<String> {
        // Run perf record for duration_secs seconds
        let perf_status = Command::new("sudo")
            .args([
                "timeout",
                "-s",
                "SIGINT",
                &duration_secs.to_string(),
                "perf",
                "record",
                "-e",
                event,
            ])
            .status()?;

        if !perf_status.success() {
            // Check if it's the expected signal termination (SIGINT from timeout)
            // timeout sends SIGINT, which will cause a non-zero exit code (130 typically)
            if !matches!(perf_status.code(), Some(124) | Some(130) | Some(143)) {
                panic!(
                    "perf record failed with exit code: {:?}",
                    perf_status.code()
                );
            }
        }

        // Change permissions on perf.data (which is the default file perf records to) to allow reading
        let chmod_status = Command::new("sudo")
            .args(["chmod", "uog+r", "./perf.data"])
            .status()?;

        if !chmod_status.success() {
            panic!("chmod failed with exit code: {:?}", chmod_status.code());
        }

        // Decode the performance data and return it directly
        // Note: This tool must be installed on the machine
        // git clone https://github.com/microsoft/LinuxTracepoints &&
        // cd LinuxTracepoints && mkdir build && cd build && cmake .. && make &&
        // sudo cp bin/perf-decode /usr/local/bin &&
        let decode_output = Command::new("perf-decode").args(["./perf.data"]).output()?;

        if !decode_output.status.success() {
            panic!(
                "perf-decode failed with exit code: {:?}",
                decode_output.status.code()
            );
        }

        // Convert the output to a String
        let raw_output = String::from_utf8_lossy(&decode_output.stdout).to_string();

        // Remove any Byte Order Mark (BOM) characters
        // UTF-8 BOM is EF BB BF (in hex)
        let cleaned_output = if let Some(stripped) = raw_output.strip_prefix('\u{FEFF}') {
            // Skip the BOM character
            stripped.to_string()
        } else {
            raw_output
        };

        // Trim the output to remove any leading/trailing whitespace
        let trimmed_output = cleaned_output.trim().to_string();

        Ok(trimmed_output)
    }
}
