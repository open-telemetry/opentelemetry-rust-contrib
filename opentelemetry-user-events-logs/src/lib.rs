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
//!     eprintln!("Failed to create user_events processor. Error: {err}");
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
//! ## Resource Attribute Handling
//!
//! **Important**: By default, resource attributes are NOT exported with log records.
//! The user_events exporter only automatically exports these specific resource attributes:
//!
//! - **`service.name`** → Exported as `cloud.roleName` in PartA of Common Schema
//! - **`service.instance.id`** → Exported as `cloud.roleInstance` in PartA of Common Schema
//!
//! All other resource attributes are ignored unless explicitly specified.
//!
//! ### Opting in to Additional Resource Attributes
//!
//! To export additional resource attributes, use the `with_resource_attributes()` method:
//!
//! ```rust
//! use opentelemetry_sdk::logs::SdkLoggerProvider;
//! use opentelemetry_sdk::Resource;
//! use opentelemetry_user_events_logs::Processor;
//! use opentelemetry::KeyValue;
//!
//! let user_event_processor = Processor::builder("myprovider")
//!     // Only export specific resource attributes
//!     .with_resource_attributes(["custom_attribute1", "custom_attribute2"])
//!     .build()
//!     .unwrap();
//!
//! let provider = SdkLoggerProvider::builder()
//!     .with_resource(
//!         Resource::builder_empty()
//!             .with_service_name("example")
//!             .with_attribute(KeyValue::new("custom_attribute1", "value1"))
//!             .with_attribute(KeyValue::new("custom_attribute2", "value2"))
//!             .with_attribute(KeyValue::new("custom_attribute2", "value3"))  // This won't be exported
//!             .build(),
//!     )
//!     .with_log_processor(user_event_processor)
//!     .build();
//! ```
//!
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

#[cfg(feature = "experimental_eventname_callback")]
pub use logs::EventNameCallback;

#[cfg(all(test, target_os = "linux"))]
mod tests {

    #[cfg(feature = "experimental_eventname_callback")]
    use crate::EventNameCallback;
    use crate::Processor;
    use one_collect::perf_event::{RingBufBuilder, RingBufSessionBuilder};
    use one_collect::tracefs::TraceFS;
    use one_collect::Writable;
    use opentelemetry::trace::Tracer;
    use opentelemetry::trace::{TraceContextExt, TracerProvider};
    use opentelemetry::{Key, KeyValue};
    use opentelemetry_appender_tracing::layer;
    use opentelemetry_sdk::Resource;
    use opentelemetry_sdk::{
        logs::LoggerProviderBuilder,
        trace::{Sampler, SdkTracerProvider},
    };
    use serde_json::{from_str, Value};
    use std::time::Duration;
    use tracepoint_decode::{EventHeaderEnumeratorContext, PerfConvertOptions};
    use tracing::error;
    use tracing_subscriber::{layer::SubscriberExt, EnvFilter, Layer};

    // This test requires a Linux kernel with user_events support. Events are
    // captured and decoded in-process via `one_collect` + `tracepoint_decode`,
    // so no external `perf`/`perf-decode` tooling is needed.
    // It is run in CI via the user-events-integration-test job.
    // To run locally: sudo -E ~/.cargo/bin/cargo test integration_test_basic -- --nocapture --ignored

    #[ignore]
    #[test]
    fn integration_test_basic() {
        // Basic check if user_events are available
        check_user_events_available().expect("Kernel does not support user_events. Verify your distribution/kernel supports user_events: https://docs.kernel.org/trace/user_events.html.");
        let user_event_processor = Processor::builder("myprovider")
            .with_resource_attributes(vec!["resource_attribute1", "resource_attribute2"])
            .build()
            .unwrap();

        let logger_provider = LoggerProviderBuilder::default()
            .with_resource(
                Resource::builder()
                    .with_service_name("myrolename")
                    .with_attribute(KeyValue::new("service.instance.id", "myinstance123"))
                    .with_attribute(KeyValue::new("resource_attribute1", "v1"))
                    .with_attribute(KeyValue::new("resource_attribute2", "v2"))
                    .with_attribute(KeyValue::new("resource_attribute3", "v3"))
                    .build(),
            )
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
            std::thread::spawn(|| capture_and_decode_events(5, "user_events:myprovider_L2K1"));

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

        let role_instance = part_a
            .get("ext_cloud_roleInstance")
            .expect("PartA.ext_cloud_roleInstance is missing");
        assert_eq!(role_instance.as_str().unwrap(), "myinstance123");

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
        assert_eq!(part_c["resource_attribute1"].as_str().unwrap(), "v1");
        assert_eq!(part_c["resource_attribute2"].as_str().unwrap(), "v2");
        assert!(
            part_c.get("resource_attribute3").is_none(),
            "resource_attribute3 should not be present"
        );
        assert_eq!(
            part_c["user_email"].as_str().unwrap(),
            "otel.user@opentelemetry.com"
        );
        assert!(part_c["bool_field"].as_bool().unwrap());
        assert_eq!(part_c["double_field"].as_f64().unwrap(), 1.0);
    }

    #[ignore]
    #[test]
    #[cfg(feature = "experimental_eventname_callback")]
    fn integration_test_callback_event_name() {
        // Run using the below command
        // sudo -E ~/.cargo/bin/cargo test integration_test_callback_event_name -- --nocapture --ignored

        #[derive(Debug, Clone)]
        struct FixedEventNameCallback;

        impl EventNameCallback for FixedEventNameCallback {
            fn get_name(&self, _: &opentelemetry_sdk::logs::SdkLogRecord) -> &'static str {
                "MyEventName"
            }
        }

        integration_test_callback_event_name_helper(
            FixedEventNameCallback,
            "myprovider:MyEventName",
        );
    }

    #[ignore]
    #[test]
    #[cfg(feature = "experimental_eventname_callback")]
    fn integration_test_callback_event_name_from_logrecord() {
        // Run using the below command
        // sudo -E ~/.cargo/bin/cargo test integration_test_callback_event_name_from_logrecord -- --nocapture --ignored

        #[derive(Debug, Clone)]
        struct TestEventNameCallback;

        impl EventNameCallback for TestEventNameCallback {
            fn get_name(&self, log_record: &opentelemetry_sdk::logs::SdkLogRecord) -> &'static str {
                log_record.event_name().unwrap_or("MyEventName")
            }
        }

        integration_test_callback_event_name_helper(
            TestEventNameCallback,
            "myprovider:my-event-name",
        );
    }

    #[cfg(feature = "experimental_eventname_callback")]
    fn integration_test_callback_event_name_helper<C>(
        event_name_callback: C,
        expected_event_name: &'static str,
    ) where
        C: EventNameCallback + 'static,
    {
        // Basic check if user_events are available
        check_user_events_available().expect("Kernel does not support user_events. Verify your distribution/kernel supports user_events: https://docs.kernel.org/trace/user_events.html.");
        let user_event_processor = Processor::builder("myprovider")
            .with_resource_attributes(vec!["resource_attribute1"])
            .with_event_name_callback(event_name_callback)
            .build()
            .unwrap();

        let logger_provider = LoggerProviderBuilder::default()
            .with_resource(
                Resource::builder()
                    .with_service_name("myrolename")
                    .with_attribute(KeyValue::new("resource_attribute1", "v1"))
                    .with_attribute(KeyValue::new("resource_attribute2", "v2"))
                    .build(),
            )
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
            std::thread::spawn(|| capture_and_decode_events(5, "user_events:myprovider_L2K1"));

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

        // Find the specific event
        let event = events
            .iter()
            .find(|e| {
                if let Some(name) = e.get("n") {
                    name.as_str().unwrap_or("") == expected_event_name
                } else {
                    false
                }
            })
            .unwrap_or_else(|| panic!("Event '{expected_event_name}' not found"));

        // Validate event structure and fields
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
        assert_eq!(part_c["resource_attribute1"].as_str().unwrap(), "v1");
        assert!(
            part_c.get("resource_attribute3").is_none(),
            "resource_attribute3 should not be present"
        );
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
            std::thread::spawn(|| capture_and_decode_events(5, "user_events:myprovider_L2K1"));

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

        // Validate trace_id and span_id (use to_string() for consistent
        // zero-padded Display formatting, matching the exporter output)
        assert_eq!(
            part_a_ext_dt_trace_id.as_str().unwrap(),
            trace_id_expected.to_string()
        );
        assert_eq!(
            part_a_ext_dt_span_id.as_str().unwrap(),
            span_id_expected.to_string()
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
    fn integration_test_direct_helper(
        severity: opentelemetry::logs::Severity,
        severity_text: &'static str,
        trace_point: &str,
    ) {
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
        record.set_severity_text(severity_text);
        record.set_event_name("my-event-name");
        record.set_target("my-target");
        record.set_body(AnyValue::from("This is a test message"));
        record.add_attribute("event_id", AnyValue::Int(99));
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
            std::thread::spawn(move || capture_and_decode_events(5, trace_point_clone.as_ref()));

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
        assert_eq!(part_b["severityText"].as_str().unwrap(), severity_text);
        assert_eq!(part_b["eventId"].as_i64().unwrap(), 99);
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
        integration_test_direct_helper(Severity::Debug, "DEBUG", "user_events:myprovider_L5K1");
        integration_test_direct_helper(Severity::Info, "INFO", "user_events:myprovider_L4K1");
        integration_test_direct_helper(Severity::Warn, "WARN", "user_events:myprovider_L3K1");
        integration_test_direct_helper(Severity::Error, "ERROR", "user_events:myprovider_L2K1");
        integration_test_direct_helper(Severity::Fatal, "FATAL", "user_events:myprovider_L1K1");
    }

    /// Test with empty resource — validates no ext_cloud fields in PartA,
    /// no PartC, and minimal PartB (body + severity only, no eventId/name).
    #[ignore]
    #[test]
    fn integration_test_no_resource() {
        use opentelemetry::logs::LogRecord;
        use opentelemetry::logs::Logger;
        use opentelemetry::logs::LoggerProvider;

        check_user_events_available().expect("Kernel does not support user_events.");

        let user_event_processor = Processor::builder("myprovider_noresource").build().unwrap();

        let logger_provider = LoggerProviderBuilder::default()
            .with_resource(Resource::builder_empty().build())
            .with_log_processor(user_event_processor)
            .build();

        let logger = logger_provider.logger("test");
        let mut record = logger.create_log_record();
        record.set_severity_number(opentelemetry::logs::Severity::Warn);
        record.set_severity_text("WARN");
        record.set_body("minimal message".into());
        // No event_name, no event_id, no attributes

        let perf_thread =
            std::thread::spawn(|| capture_and_decode_events(5, "user_events:myprovider_noresource_L3K1"));

        std::thread::sleep(std::time::Duration::from_millis(1000));

        logger.emit(record);

        let result = perf_thread.join().expect("Perf thread panicked");
        assert!(result.is_ok());
        let json_content = result.unwrap();
        assert!(!json_content.is_empty());

        let formatted_output = json_content.trim().to_string();
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

        let event = events
            .iter()
            .find(|e| e.get("n").and_then(|n| n.as_str()) == Some("myprovider_noresource:Log"))
            .expect("Event 'myprovider_noresource:Log' not found");

        assert_eq!(event["__csver__"].as_i64().unwrap(), 1024);

        // PartA — no ext_cloud_role or ext_cloud_roleInstance
        let part_a = &event["PartA"];
        assert!(part_a.get("time").is_some(), "PartA.time is missing");
        assert!(
            part_a.get("ext_cloud_role").is_none(),
            "ext_cloud_role should not be present with empty resource"
        );
        assert!(
            part_a.get("ext_cloud_roleInstance").is_none(),
            "ext_cloud_roleInstance should not be present with empty resource"
        );

        // No PartC expected (no attributes)
        assert!(
            event.get("PartC").is_none(),
            "PartC should not be present when there are no attributes"
        );

        // PartB — minimal fields
        let part_b = &event["PartB"];
        assert_eq!(part_b["_typeName"].as_str().unwrap(), "Log");
        assert_eq!(part_b["body"].as_str().unwrap(), "minimal message");
        assert_eq!(
            part_b["severityNumber"].as_i64().unwrap(),
            opentelemetry::logs::Severity::Warn as i64
        );
        assert_eq!(part_b["severityText"].as_str().unwrap(), "WARN");
        // No eventId or name fields should be present
        assert!(
            part_b.get("eventId").is_none(),
            "eventId should not be present"
        );
        assert!(part_b.get("name").is_none(), "name should not be present");
    }

    fn check_user_events_available() -> Result<String, String> {
        // Read the list of currently-registered user_events tracepoints directly
        // from tracefs (`<tracefs>/user_events_status`) instead of shelling out
        // to `sudo cat`. The test process already runs elevated, so it can read
        // the status file directly.
        let tracefs = TraceFS::open().map_err(|e| {
            format!(
                "Unable to open tracefs. user_events requires a Linux kernel with \
                 tracefs mounted and sufficient permissions \
                 (https://docs.kernel.org/trace/user_events.html): {e}"
            )
        })?;
        let status_path = tracefs
            .user_events_path()
            .with_file_name("user_events_status");
        std::fs::read_to_string(&status_path)
            .map_err(|e| format!("Failed to read {}: {e}", status_path.display()))
    }

    /// Captures EventHeader events from the given user_events tracepoint in-process
    /// using `one_collect` for the perf ring buffer session and `tracepoint_decode`
    /// for EventHeader decoding, then returns the decoded events as a JSON string
    /// in the same `{ "./perf.data": [ {event}, ... ] }` shape the old
    /// `perf-decode` pipeline produced, so the test assertions are unchanged.
    ///
    /// Returns an error if any captured record fails to decode, so decode
    /// failures surface as a test failure rather than being silently dropped.
    ///
    /// `event` is the full tracepoint spec, e.g. "user_events:myprovider_L2K1".
    /// Capture runs for `duration_secs`; the caller emits events on another thread
    /// during this window, mirroring the old `perf record` duration.
    ///
    /// This replaces the previous `perf record` + `perf-decode` external-tool
    /// pipeline with a self-contained, in-process consumer (no external tools, no
    /// temp files, no `sudo` shell-outs).
    fn capture_and_decode_events(duration_secs: u64, event: &str) -> std::io::Result<String> {
        let need_permission = "Need permission to access tracefs/perf_events (run via sudo?)";

        // Strip the "user_events:" system prefix to get the tracepoint name,
        // e.g. "user_events:myprovider_L2K1" -> "myprovider_L2K1".
        let tracepoint = event
            .strip_prefix("user_events:")
            .unwrap_or(event)
            .to_string();

        let tracefs = TraceFS::open()?;
        let mut tp_event = tracefs.find_event("user_events", &tracepoint)?;

        // The EventHeader payload begins at the `eventheader_flags` field, right
        // after the tracepoint common fields. `tracepoint_decode` wants the event
        // data starting at that offset.
        let flags_offset = tp_event
            .format()
            .get_field("eventheader_flags")
            .map(|f| f.offset)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "eventheader_flags field missing from tracepoint format",
                )
            })?;

        let tp_name = tracepoint.clone();
        let collected = Writable::<Vec<String>>::new(Vec::new());
        let errors = Writable::<Vec<String>>::new(Vec::new());
        let sink = collected.clone();
        let err_sink = errors.clone();
        let mut ctx = EventHeaderEnumeratorContext::new();

        tp_event.add_callback(move |data| {
            let full = data.event_data();
            if full.len() <= flags_offset {
                return Ok(());
            }
            let payload = &full[flags_offset..];

            // Capture decode failures (rather than silently dropping the event)
            // so the helper can surface them and the test fails with a
            // diagnosable error instead of a missing/empty result.
            let mut enumerator = match ctx.enumerate_with_name_and_data(
                &tp_name,
                payload,
                EventHeaderEnumeratorContext::MOVE_NEXT_LIMIT_DEFAULT,
            ) {
                Ok(enumerator) => enumerator,
                Err(e) => {
                    err_sink
                        .write(|errs| errs.push(format!("failed to start EventHeader decode: {e}")));
                    return Ok(());
                }
            };

            // Event identity, e.g. "myprovider:Log".
            let identity = enumerator.event_info().identity_display().to_string();

            // From the initial BeforeFirstItem state, this appends every
            // top-level field as a comma-separated list of `"Name": value`
            // pairs, with PartA/PartB/PartC rendered as nested JSON objects.
            let mut body = String::new();
            match enumerator.write_json_item_and_move_next_sibling(
                &mut body,
                false,
                PerfConvertOptions::Default,
            ) {
                Ok(_) => sink.write(|out| out.push(format!("{{\"n\":\"{identity}\",{body}}}"))),
                Err(e) => err_sink.write(|errs| {
                    errs.push(format!("failed to write JSON for event {identity}: {e}"))
                }),
            }
            Ok(())
        });

        let mut session = RingBufSessionBuilder::new()
            .with_page_count(32)
            .with_tracepoint_events(RingBufBuilder::for_tracepoint())
            .with_target_pid(std::process::id() as i32)
            .build()
            .expect(need_permission);

        session
            .add_event(tp_event)
            .expect("Failed to add tracepoint event to session");
        session.enable().expect(need_permission);

        // Capture for the requested window. The caller emits events on another
        // thread during this time; `parse_for_duration` keeps draining the ring
        // buffer until the duration elapses, so mid-window writes are captured.
        session
            .parse_for_duration(Duration::from_secs(duration_secs))
            .expect("Failed to parse perf ring buffer");
        session.disable().expect(need_permission);

        // Surface any decode failures instead of silently dropping them, so a
        // malformed event makes the test fail with a diagnosable error rather
        // than a missing/short result.
        let mut decode_errors = Vec::new();
        errors.read(|v| decode_errors = v.clone());
        if !decode_errors.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Failed to decode {} user_events record(s): {}",
                    decode_errors.len(),
                    decode_errors.join("; ")
                ),
            ));
        }

        let mut events = Vec::new();
        collected.read(|v| events = v.clone());

        Ok(format!("{{\"./perf.data\":[{}]}}", events.join(",")))
    }
}
