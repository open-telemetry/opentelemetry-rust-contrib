//! The user_events exporter will enable applications to use OpenTelemetry API
//! to capture the telemetry events, and write to user_events subsystem.
//!
//! # Resource Attribute Mapping
//!
//! The following OpenTelemetry resource attributes are automatically mapped
//! to fields in the exported event:
//!
//! | Resource Attribute      | Exported Field           |
//! |-------------------------|--------------------------|
//! | `service.name`          | `ext_cloud_role`         |
//! | `service.instance.id`   | `ext_cloud_roleInstance` |
//!
//! These are set via the [`Resource`](opentelemetry_sdk::Resource) on the
//! [`SdkTracerProvider`](opentelemetry_sdk::trace::SdkTracerProvider):
//!
//! ```no_run
//! use opentelemetry::KeyValue;
//! use opentelemetry_sdk::trace::SdkTracerProvider;
//! use opentelemetry_user_events_trace::UserEventsTracerProviderBuilderExt;
//!
//! let provider = SdkTracerProvider::builder()
//!     .with_resource(
//!         opentelemetry_sdk::Resource::builder()
//!             .with_service_name("my-service")
//!             .with_attribute(KeyValue::new("service.instance.id", "instance-1"))
//!             .build(),
//!     )
//!     .with_user_events_exporter("my_provider")
//!     .build();
//! ```
//!
//! # Well-Known Span Attributes
//!
//! Certain span attributes are recognized as "well-known" and mapped to
//! dedicated fields in the exported event:
//!
//! | Span Attribute              | Exported Field         |
//! |-----------------------------|------------------------|
//! | `db.system.name`            | `dbSystem`             |
//! | `db.namespace`              | `dbName`               |
//! | `db.query.text`             | `dbStatement`          |
//! | `http.request.method`       | `httpMethod`           |
//! | `url.full`                  | `httpUrl`              |
//! | `http.response.status_code` | `httpStatusCode`       |
//! | `messaging.system`          | `messagingSystem`      |
//! | `messaging.destination.name`| `messagingDestination` |
//! | `messaging.url`             | `messagingUrl`         |
//! | `rpc.system.name`           | `rpcSystem`            |
//! | `rpc.response.status_code`  | `rpcGrpcStatusCode`    |
//!
//! All other span attributes are exported with their original keys.

#![warn(missing_debug_implementations, missing_docs)]

mod trace;

pub use trace::*;

#[cfg(all(test, target_os = "linux"))]
mod tests {

    use crate::UserEventsTracerProviderBuilderExt;
    use one_collect::perf_event::{RingBufBuilder, RingBufSessionBuilder};
    use one_collect::tracefs::TraceFS;
    use one_collect::Writable;
    use opentelemetry::{
        trace::{TraceContextExt, Tracer, TracerProvider},
        KeyValue,
    };
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use serde_json::{from_str, Value};
    use std::time::Duration;
    use tracepoint_decode::{EventHeaderEnumeratorContext, PerfConvertOptions};

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
        let provider = SdkTracerProvider::builder()
            .with_resource(
                opentelemetry_sdk::Resource::builder()
                    .with_service_name("myrolename")
                    .with_attribute(KeyValue::new("service.instance.id", "myinstance123"))
                    .build(),
            )
            .with_user_events_exporter("opentelemetry_traces")
            .build();

        // Validate that the TracePoint is created.
        // There is no notion of Severity for Spans in OTel,
        // so we use the default severity level of 4 (Info).
        let user_event_status = check_user_events_available().expect("Kernel does not support user_events. Verify your distribution/kernel supports user_events: https://docs.kernel.org/trace/user_events.html.");
        assert!(user_event_status.contains("opentelemetry_traces_L4K1"));

        // Start perf recording in a separate thread and emit logs in parallel.
        let perf_thread =
            std::thread::spawn(|| capture_and_decode_events(5, "user_events:opentelemetry_traces_L4K1"));

        // Give a little time for perf to start recording
        std::thread::sleep(std::time::Duration::from_millis(1000));

        // ACT
        let tracer = provider.tracer("user-events-tracer");
        let (trace_id_expected, span_id_expected) = tracer.in_span("my-span-name", |cx| {
            let span = cx.span();
            let trace_id = span.span_context().trace_id();
            let span_id = span.span_context().span_id();

            // Set PartB attributes
            // Database attributes (stable semconv keys)
            span.set_attribute(KeyValue::new("db.system.name", "postgresql"));
            span.set_attribute(KeyValue::new("db.namespace", "inventory"));
            span.set_attribute(KeyValue::new(
                "db.query.text",
                "SELECT * FROM products WHERE price > 100",
            ));
            // HTTP attributes
            span.set_attribute(KeyValue::new("http.request.method", "GET"));
            span.set_attribute(KeyValue::new(
                "url.full",
                "https://api.example.com/products?min_price=100",
            ));
            span.set_attribute(KeyValue::new("http.response.status_code", 200));
            // Messaging attributes (stable semconv keys)
            span.set_attribute(KeyValue::new("messaging.system", "kafka"));
            span.set_attribute(KeyValue::new(
                "messaging.destination.name",
                "product-updates",
            ));
            span.set_attribute(KeyValue::new(
                "messaging.url",
                "kafka://broker1.example.com:9092",
            ));

            // Set PartC attributes
            span.set_attribute(KeyValue::new("my-key", "my-value"));

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
        { "n": "opentelemetry_traces:Span", "__csver__": 1024, "PartA": { "time": "2025-07-10T23:04:08.109839907+00:00", "ext_dt_traceId": "e8bbbe6db41c807792b93648ad9398e1", "ext_dt_spanId": "cfdb9dc3c3948453", "ext_cloud_role": "user-events-trace-example" }, "PartB": { "_typeName": "Span", "name": "my-span-name", "parentId": "", "startTime": "2025-07-10T23:04:08.109797282+00:00", "success": true, "kind": 0 }, "PartC": { "my-key": "my-value" }, "meta": { "time": 104590.077244551, "cpu": 7, "pid": 176542, "tid": 176542, "level": 4, "keyword": "0x1" } } ]
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
                    name.as_str().unwrap_or("") == "opentelemetry_traces:Span"
                } else {
                    false
                }
            })
            .expect("Event 'opentelemetry_traces:Span' not found");

        // Validate event structure and fields
        assert_eq!(event["n"].as_str().unwrap(), "opentelemetry_traces:Span");
        assert_eq!(event["__csver__"].as_i64().unwrap(), 1024);

        // Validate PartA
        let part_a = &event["PartA"];
        let time_str = part_a["time"].as_str().expect("PartA.time is missing");
        assert!(
            time_str.ends_with('Z'),
            "PartA.time should end with 'Z', got: {time_str}"
        );

        let part_a_ext_dt_trace_id = part_a
            .get("ext_dt_traceId")
            .expect("PartA.ext_dt_traceId is missing");
        let part_a_ext_dt_span_id = part_a
            .get("ext_dt_spanId")
            .expect("PartA.ext_dt_spanId is missing");

        // Validate trace_id and span_id
        assert_eq!(
            part_a_ext_dt_trace_id.as_str().unwrap(),
            trace_id_expected.to_string()
        );
        assert_eq!(
            part_a_ext_dt_span_id.as_str().unwrap(),
            span_id_expected.to_string()
        );

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
        assert_eq!(part_b["_typeName"].as_str().unwrap(), "Span");
        assert_eq!(part_b["name"].as_str().unwrap(), "my-span-name");
        // Root span has no parent, so parentId should not be present
        assert!(
            part_b.get("parentId").is_none(),
            "parentId should not be present for root span"
        );
        let start_time_str = part_b["startTime"]
            .as_str()
            .expect("PartB.startTime is missing");
        assert!(
            start_time_str.ends_with('Z'),
            "PartB.startTime should end with 'Z', got: {start_time_str}"
        );
        assert!(part_b["success"].as_bool().unwrap());
        assert_eq!(part_b["kind"].as_i64().unwrap(), 0);

        // Validate attributes that become PartB
        assert_eq!(part_b["dbSystem"].as_str().unwrap(), "postgresql");
        assert_eq!(part_b["dbName"].as_str().unwrap(), "inventory");
        assert_eq!(
            part_b["dbStatement"].as_str().unwrap(),
            "SELECT * FROM products WHERE price > 100"
        );
        assert_eq!(part_b["httpMethod"].as_str().unwrap(), "GET");
        assert_eq!(
            part_b["httpUrl"].as_str().unwrap(),
            "https://api.example.com/products?min_price=100"
        );
        assert_eq!(part_b["httpStatusCode"].as_i64().unwrap(), 200);
        assert_eq!(part_b["messagingSystem"].as_str().unwrap(), "kafka");
        assert_eq!(
            part_b["messagingDestination"].as_str().unwrap(),
            "product-updates"
        );
        assert_eq!(
            part_b["messagingUrl"].as_str().unwrap(),
            "kafka://broker1.example.com:9092"
        );

        // Validate PartC
        let part_c = &event["PartC"];
        assert_eq!(part_c["my-key"].as_str().unwrap(), "my-value");
    }

    /// Test with a child span that has Error status and Client SpanKind.
    /// Validates parentId serialization, success=false, kind=2, and
    /// non-string PartC attribute types (bool, f64).
    #[ignore]
    #[test]
    fn integration_test_child_span_with_error() {
        use opentelemetry::trace::{Span, SpanKind, Status};

        check_user_events_available().expect("Kernel does not support user_events.");

        let provider = SdkTracerProvider::builder()
            .with_resource(
                opentelemetry_sdk::Resource::builder()
                    .with_service_name("child_span_test")
                    .build(),
            )
            .with_user_events_exporter("otel_trace_child")
            .build();

        let user_event_status = check_user_events_available().unwrap();
        assert!(user_event_status.contains("otel_trace_child_L4K1"));

        let perf_thread =
            std::thread::spawn(|| capture_and_decode_events(5, "user_events:otel_trace_child_L4K1"));

        std::thread::sleep(std::time::Duration::from_millis(1000));

        // ACT — create a parent span, then a child span with error status
        let tracer = provider.tracer("test-tracer");

        let (parent_span_id, child_trace_id, child_span_id) = {
            let parent = tracer
                .span_builder("parent-span")
                .with_kind(SpanKind::Server)
                .start(&tracer);
            let parent_cx = opentelemetry::Context::current_with_span(parent);
            let parent_span_id = parent_cx.span().span_context().span_id();

            let mut child = tracer
                .span_builder("child-span")
                .with_kind(SpanKind::Client)
                .start_with_context(&tracer, &parent_cx);
            let child_trace_id = child.span_context().trace_id();
            let child_span_id = child.span_context().span_id();

            child.set_attribute(KeyValue::new("bool_attr", true));
            child.set_attribute(KeyValue::new("float_attr", 1.5));
            child.set_status(Status::error("something went wrong"));
            child.end();

            // Parent ends after child — both get exported, but we look for the child.
            drop(parent_cx); // ends parent span

            (parent_span_id, child_trace_id, child_span_id)
        };

        let result = perf_thread.join().expect("Perf thread panicked");
        assert!(result.is_ok());
        let formatted_output = result.unwrap().trim().to_string();

        let json_value: Value = from_str(&formatted_output).expect("Failed to parse JSON");
        let perf_data_key = json_value
            .as_object()
            .unwrap()
            .keys()
            .find(|k| k.contains("perf.data"))
            .expect("No perf.data key found");

        let events = json_value[perf_data_key].as_array().unwrap();

        // Find the child span event by matching its spanId
        let child_span_id_str = child_span_id.to_string();
        let event = events
            .iter()
            .find(|e| {
                e.get("PartA")
                    .and_then(|a| a.get("ext_dt_spanId"))
                    .and_then(|s| s.as_str())
                    == Some(&child_span_id_str)
            })
            .expect("Child span event not found");

        // Validate PartA
        let part_a = &event["PartA"];
        assert!(part_a["time"].as_str().unwrap().ends_with('Z'));
        assert_eq!(
            part_a["ext_dt_traceId"].as_str().unwrap(),
            child_trace_id.to_string()
        );
        assert_eq!(
            part_a["ext_cloud_role"].as_str().unwrap(),
            "child_span_test"
        );

        // Validate PartB
        let part_b = &event["PartB"];
        assert_eq!(part_b["_typeName"].as_str().unwrap(), "Span");
        assert_eq!(part_b["name"].as_str().unwrap(), "child-span");
        assert!(part_b["startTime"].as_str().unwrap().ends_with('Z'));

        // success should be false (Error status)
        assert!(!part_b["success"].as_bool().unwrap());

        // kind should be 2 (Client)
        assert_eq!(part_b["kind"].as_i64().unwrap(), 2);

        // parentId should be present and match the parent span
        let parent_id = part_b
            .get("parentId")
            .expect("PartB.parentId should be present for child span");
        assert_eq!(parent_id.as_str().unwrap(), parent_span_id.to_string());

        // statusMessage should be present for error span with description
        assert_eq!(
            part_b["statusMessage"].as_str().unwrap(),
            "something went wrong"
        );

        // Validate PartC — non-string attribute types
        let part_c = &event["PartC"];
        assert!(part_c["bool_attr"].as_bool().unwrap());
        assert!((part_c["float_attr"].as_f64().unwrap() - 1.5).abs() < 0.001);
    }

    /// Test with empty resource — validates no ext_cloud fields in PartA.
    #[ignore]
    #[test]
    fn integration_test_no_resource() {
        check_user_events_available().expect("Kernel does not support user_events.");

        let provider = SdkTracerProvider::builder()
            .with_resource(opentelemetry_sdk::Resource::builder_empty().build())
            .with_user_events_exporter("otel_trace_nores")
            .build();

        let user_event_status = check_user_events_available().unwrap();
        assert!(user_event_status.contains("otel_trace_nores_L4K1"));

        let perf_thread =
            std::thread::spawn(|| capture_and_decode_events(5, "user_events:otel_trace_nores_L4K1"));

        std::thread::sleep(std::time::Duration::from_millis(1000));

        let tracer = provider.tracer("test-tracer");
        let span_id_expected =
            tracer.in_span("no-resource-span", |cx| cx.span().span_context().span_id());

        let result = perf_thread.join().expect("Perf thread panicked");
        assert!(result.is_ok());
        let formatted_output = result.unwrap().trim().to_string();

        let json_value: Value = from_str(&formatted_output).expect("Failed to parse JSON");
        let perf_data_key = json_value
            .as_object()
            .unwrap()
            .keys()
            .find(|k| k.contains("perf.data"))
            .expect("No perf.data key found");

        let events = json_value[perf_data_key].as_array().unwrap();

        let event = events
            .iter()
            .find(|e| e.get("n").and_then(|n| n.as_str()) == Some("otel_trace_nores:Span"))
            .expect("Event 'otel_trace_nores:Span' not found");

        assert_eq!(event["__csver__"].as_i64().unwrap(), 1024);

        // PartA — no ext_cloud_role or ext_cloud_roleInstance
        let part_a = &event["PartA"];
        assert!(part_a["time"].as_str().unwrap().ends_with('Z'));
        assert!(part_a.get("ext_dt_traceId").is_some());
        assert_eq!(
            part_a["ext_dt_spanId"].as_str().unwrap(),
            span_id_expected.to_string()
        );
        assert!(
            part_a.get("ext_cloud_role").is_none(),
            "ext_cloud_role should not be present with empty resource"
        );
        assert!(
            part_a.get("ext_cloud_roleInstance").is_none(),
            "ext_cloud_roleInstance should not be present with empty resource"
        );

        // PartB — minimal
        let part_b = &event["PartB"];
        assert_eq!(part_b["_typeName"].as_str().unwrap(), "Span");
        assert_eq!(part_b["name"].as_str().unwrap(), "no-resource-span");
        assert!(part_b["success"].as_bool().unwrap());
        assert_eq!(part_b["kind"].as_i64().unwrap(), 0); // Internal
        assert!(
            part_b.get("parentId").is_none(),
            "parentId should not be present for root span"
        );

        // No PartC expected (no custom attributes)
        assert!(
            event.get("PartC").is_none(),
            "PartC should not be present when there are no attributes"
        );
    }

    /// Test span links serialization.
    /// Creates a span with links to another span context.
    #[ignore]
    #[test]
    fn integration_test_links() {
        use opentelemetry::trace::{
            Link, Span, SpanContext, SpanId, SpanKind, TraceFlags, TraceId, TraceState,
        };

        check_user_events_available().expect("Kernel does not support user_events.");

        let provider = SdkTracerProvider::builder()
            .with_resource(
                opentelemetry_sdk::Resource::builder()
                    .with_service_name("links_test")
                    .build(),
            )
            .with_user_events_exporter("otel_trace_links")
            .build();

        let user_event_status = check_user_events_available().unwrap();
        assert!(user_event_status.contains("otel_trace_links_L4K1"));

        let perf_thread =
            std::thread::spawn(|| capture_and_decode_events(5, "user_events:otel_trace_links_L4K1"));

        std::thread::sleep(std::time::Duration::from_millis(1000));

        let tracer = provider.tracer("test-tracer");

        // Create a linked span context (simulating a link to another trace)
        let linked_trace_id = TraceId::from_hex("0af7651916cd43dd8448eb211c80319c").unwrap();
        let linked_span_id = SpanId::from_hex("00f067aa0ba902b7").unwrap();
        let linked_context = SpanContext::new(
            linked_trace_id,
            linked_span_id,
            TraceFlags::SAMPLED,
            false,
            TraceState::default(),
        );

        let span_id_expected = {
            let mut span = tracer
                .span_builder("span-with-links")
                .with_kind(SpanKind::Internal)
                .with_links(vec![Link::new(linked_context, vec![], 0)])
                .start(&tracer);
            let sid = span.span_context().span_id();
            span.end();
            sid
        };

        let result = perf_thread.join().expect("Perf thread panicked");
        assert!(result.is_ok());
        let formatted_output = result.unwrap().trim().to_string();

        let json_value: Value = from_str(&formatted_output).expect("Failed to parse JSON");
        let perf_data_key = json_value
            .as_object()
            .unwrap()
            .keys()
            .find(|k| k.contains("perf.data"))
            .expect("No perf.data key found");

        let events = json_value[perf_data_key].as_array().unwrap();

        let span_id_str = span_id_expected.to_string();
        let event = events
            .iter()
            .find(|e| {
                e.get("PartA")
                    .and_then(|a| a.get("ext_dt_spanId"))
                    .and_then(|s| s.as_str())
                    == Some(&span_id_str)
            })
            .expect("Span event not found");

        // Validate PartB
        let part_b = &event["PartB"];
        assert_eq!(part_b["name"].as_str().unwrap(), "span-with-links");
        assert!(part_b["success"].as_bool().unwrap());

        // links should be present as JSON string
        let links_str = part_b
            .get("links")
            .expect("PartB.links should be present")
            .as_str()
            .expect("links should be a string");
        let links: Vec<Value> =
            serde_json::from_str(links_str).expect("links should be valid JSON");
        assert_eq!(links.len(), 1);
        assert_eq!(
            links[0]["toTraceId"].as_str().unwrap(),
            "0af7651916cd43dd8448eb211c80319c"
        );
        assert_eq!(links[0]["toSpanId"].as_str().unwrap(), "00f067aa0ba902b7");

        // statusMessage should not be present (no error)
        assert!(
            part_b.get("statusMessage").is_none(),
            "statusMessage should not be present for successful span"
        );
    }

    /// Test statusMessage serialization for error spans.
    #[ignore]
    #[test]
    fn integration_test_status_message() {
        use opentelemetry::trace::{Span, SpanKind, Status};
        use std::time::{Duration, UNIX_EPOCH};

        check_user_events_available().expect("Kernel does not support user_events.");

        let provider = SdkTracerProvider::builder()
            .with_resource(
                opentelemetry_sdk::Resource::builder()
                    .with_service_name("statusmsg_test")
                    .build(),
            )
            .with_user_events_exporter("otel_trace_stmsg")
            .build();

        let user_event_status = check_user_events_available().unwrap();
        assert!(user_event_status.contains("otel_trace_stmsg_L4K1"));

        let perf_thread =
            std::thread::spawn(|| capture_and_decode_events(5, "user_events:otel_trace_stmsg_L4K1"));

        std::thread::sleep(std::time::Duration::from_millis(1000));

        let tracer = provider.tracer("test-tracer");

        // Use a well-known start time so we can assert the exact formatted value.
        // 1_700_000_000 seconds since UNIX epoch = 2023-11-14T22:13:20Z
        let known_start_time = UNIX_EPOCH + Duration::from_secs(1_700_000_000);

        let span_id_expected = {
            let mut span = tracer
                .span_builder("error-span")
                .with_kind(SpanKind::Client)
                .with_start_time(known_start_time)
                .start(&tracer);
            let sid = span.span_context().span_id();
            span.set_status(Status::error("something went wrong"));
            span.end();
            sid
        };

        let result = perf_thread.join().expect("Perf thread panicked");
        assert!(result.is_ok());
        let formatted_output = result.unwrap().trim().to_string();

        let json_value: Value = from_str(&formatted_output).expect("Failed to parse JSON");
        let perf_data_key = json_value
            .as_object()
            .unwrap()
            .keys()
            .find(|k| k.contains("perf.data"))
            .expect("No perf.data key found");

        let events = json_value[perf_data_key].as_array().unwrap();

        let span_id_str = span_id_expected.to_string();
        let event = events
            .iter()
            .find(|e| {
                e.get("PartA")
                    .and_then(|a| a.get("ext_dt_spanId"))
                    .and_then(|s| s.as_str())
                    == Some(&span_id_str)
            })
            .expect("Span event not found");

        let part_b = &event["PartB"];
        assert_eq!(part_b["name"].as_str().unwrap(), "error-span");
        assert!(!part_b["success"].as_bool().unwrap());

        // startTime should match the known start time we provided
        assert_eq!(
            part_b["startTime"].as_str().unwrap(),
            "2023-11-14T22:13:20Z"
        );

        // statusMessage should be present for Error status with description
        let status_msg = part_b
            .get("statusMessage")
            .expect("PartB.statusMessage should be present for error span");
        assert_eq!(status_msg.as_str().unwrap(), "something went wrong");

        // links should not be present (no links)
        assert!(
            part_b.get("links").is_none(),
            "links should not be present when span has no links"
        );
    }

    /// Test that statusMessage is suppressed when httpStatusCode is present.
    /// Per Common Schema spec: "If you report httpStatusCode, statusMessage
    /// should not be reported and will be ignored."
    #[ignore]
    #[test]
    fn integration_test_status_message_suppressed_with_http_status_code() {
        use opentelemetry::trace::{Span, SpanKind, Status};

        check_user_events_available().expect("Kernel does not support user_events.");

        let provider = SdkTracerProvider::builder()
            .with_resource(
                opentelemetry_sdk::Resource::builder()
                    .with_service_name("suppress_test")
                    .build(),
            )
            .with_user_events_exporter("otel_trace_suppr")
            .build();

        let user_event_status = check_user_events_available().unwrap();
        assert!(user_event_status.contains("otel_trace_suppr_L4K1"));

        let perf_thread =
            std::thread::spawn(|| capture_and_decode_events(5, "user_events:otel_trace_suppr_L4K1"));

        std::thread::sleep(std::time::Duration::from_millis(1000));

        let tracer = provider.tracer("test-tracer");

        let span_id_expected = {
            let mut span = tracer
                .span_builder("http-error-span")
                .with_kind(SpanKind::Client)
                .start(&tracer);
            let sid = span.span_context().span_id();
            span.set_attribute(KeyValue::new("http.response.status_code", 500));
            span.set_status(Status::error("Internal Server Error"));
            span.end();
            sid
        };

        let result = perf_thread.join().expect("Perf thread panicked");
        assert!(result.is_ok());
        let formatted_output = result.unwrap().trim().to_string();

        let json_value: Value = from_str(&formatted_output).expect("Failed to parse JSON");
        let perf_data_key = json_value
            .as_object()
            .unwrap()
            .keys()
            .find(|k| k.contains("perf.data"))
            .expect("No perf.data key found");

        let events = json_value[perf_data_key].as_array().unwrap();

        let span_id_str = span_id_expected.to_string();
        let event = events
            .iter()
            .find(|e| {
                e.get("PartA")
                    .and_then(|a| a.get("ext_dt_spanId"))
                    .and_then(|s| s.as_str())
                    == Some(&span_id_str)
            })
            .expect("Span event not found");

        let part_b = &event["PartB"];
        assert!(!part_b["success"].as_bool().unwrap());

        // httpStatusCode should be in PartB as a well-known attribute
        assert_eq!(part_b["httpStatusCode"].as_i64().unwrap(), 500);

        // statusMessage should NOT be present when httpStatusCode is reported
        assert!(
            part_b.get("statusMessage").is_none(),
            "statusMessage should be suppressed when httpStatusCode is present"
        );
    }

    /// Test all SpanKind values and RPC well-known attributes.
    /// Emits 5 spans (one per SpanKind) in a single perf capture.
    #[ignore]
    #[test]
    fn integration_test_all_span_kinds_and_rpc_attrs() {
        use opentelemetry::trace::{Span, SpanKind};

        check_user_events_available().expect("Kernel does not support user_events.");

        let provider = SdkTracerProvider::builder()
            .with_resource(
                opentelemetry_sdk::Resource::builder()
                    .with_service_name("kinds_test")
                    .build(),
            )
            .with_user_events_exporter("otel_trace_kinds")
            .build();

        let user_event_status = check_user_events_available().unwrap();
        assert!(user_event_status.contains("otel_trace_kinds_L4K1"));

        let perf_thread =
            std::thread::spawn(|| capture_and_decode_events(5, "user_events:otel_trace_kinds_L4K1"));

        std::thread::sleep(std::time::Duration::from_millis(1000));

        let tracer = provider.tracer("test-tracer");

        // Emit one span per SpanKind
        let kinds: Vec<(SpanKind, &str, i64)> = vec![
            (SpanKind::Internal, "internal-span", 0),
            (SpanKind::Server, "server-span", 1),
            (SpanKind::Client, "client-span", 2),
            (SpanKind::Producer, "producer-span", 3),
            (SpanKind::Consumer, "consumer-span", 4),
        ];

        let mut span_ids: Vec<(String, &str, i64)> = Vec::new();
        for (kind, name, expected_kind) in &kinds {
            let mut span = tracer
                .span_builder(*name)
                .with_kind(kind.clone())
                .start(&tracer);

            // Add RPC attributes to the client span to test rpcSystem/rpcGrpcStatusCode
            if *name == "client-span" {
                span.set_attribute(KeyValue::new("rpc.system.name", "grpc"));
                span.set_attribute(KeyValue::new("rpc.response.status_code", 0_i64));
            }

            let sid = span.span_context().span_id().to_string();
            span.end();
            span_ids.push((sid, name, *expected_kind));
        }

        let result = perf_thread.join().expect("Perf thread panicked");
        assert!(result.is_ok());
        let formatted_output = result.unwrap().trim().to_string();

        let json_value: Value = from_str(&formatted_output).expect("Failed to parse JSON");
        let perf_data_key = json_value
            .as_object()
            .unwrap()
            .keys()
            .find(|k| k.contains("perf.data"))
            .expect("No perf.data key found");

        let events = json_value[perf_data_key].as_array().unwrap();

        for (span_id, name, expected_kind) in &span_ids {
            let event = events
                .iter()
                .find(|e| {
                    e.get("PartA")
                        .and_then(|a| a.get("ext_dt_spanId"))
                        .and_then(|s| s.as_str())
                        == Some(span_id.as_str())
                })
                .unwrap_or_else(|| panic!("Span '{name}' not found"));

            let part_b = &event["PartB"];
            assert_eq!(part_b["name"].as_str().unwrap(), *name);
            assert_eq!(
                part_b["kind"].as_i64().unwrap(),
                *expected_kind,
                "Wrong kind for span '{name}'"
            );

            // Validate RPC well-known attributes on the client span
            if *name == "client-span" {
                assert_eq!(part_b["rpcSystem"].as_str().unwrap(), "grpc");
                assert_eq!(part_b["rpcGrpcStatusCode"].as_i64().unwrap(), 0);
            }
        }
    }

    /// Test that only service.name is set (no service.instance.id).
    /// Validates ext_cloud_role is present but ext_cloud_roleInstance is absent.
    #[ignore]
    #[test]
    fn integration_test_partial_resource() {
        use opentelemetry::trace::Span;

        check_user_events_available().expect("Kernel does not support user_events.");

        let provider = SdkTracerProvider::builder()
            .with_resource(
                opentelemetry_sdk::Resource::builder()
                    .with_service_name("partial_resource_test")
                    .build(),
            )
            .with_user_events_exporter("otel_trace_pres")
            .build();

        let user_event_status = check_user_events_available().unwrap();
        assert!(user_event_status.contains("otel_trace_pres_L4K1"));

        let perf_thread =
            std::thread::spawn(|| capture_and_decode_events(5, "user_events:otel_trace_pres_L4K1"));

        std::thread::sleep(std::time::Duration::from_millis(1000));

        let tracer = provider.tracer("test-tracer");

        let span_id_expected = {
            let mut span = tracer.span_builder("partial-resource-span").start(&tracer);
            let sid = span.span_context().span_id();
            // Add only non-well-known attributes to ensure PartC works without PartB well-known attrs
            span.set_attribute(KeyValue::new("custom_i64", 42_i64));
            span.set_attribute(KeyValue::new("custom_string", "hello"));
            span.end();
            sid
        };

        let result = perf_thread.join().expect("Perf thread panicked");
        assert!(result.is_ok());
        let formatted_output = result.unwrap().trim().to_string();

        let json_value: Value = from_str(&formatted_output).expect("Failed to parse JSON");
        let perf_data_key = json_value
            .as_object()
            .unwrap()
            .keys()
            .find(|k| k.contains("perf.data"))
            .expect("No perf.data key found");

        let events = json_value[perf_data_key].as_array().unwrap();

        let span_id_str = span_id_expected.to_string();
        let event = events
            .iter()
            .find(|e| {
                e.get("PartA")
                    .and_then(|a| a.get("ext_dt_spanId"))
                    .and_then(|s| s.as_str())
                    == Some(&span_id_str)
            })
            .expect("Span event not found");

        let part_a = &event["PartA"];
        // service.name is set so ext_cloud_role should be present
        assert_eq!(
            part_a["ext_cloud_role"].as_str().unwrap(),
            "partial_resource_test"
        );
        // service.instance.id not set, so ext_cloud_roleInstance should be absent
        assert!(
            part_a.get("ext_cloud_roleInstance").is_none(),
            "ext_cloud_roleInstance should not be present when service.instance.id is not set"
        );

        // Validate PartC has the custom attributes with correct types
        let part_c = event
            .get("PartC")
            .expect("PartC should be present with custom attributes");
        assert_eq!(part_c["custom_i64"].as_i64().unwrap(), 42);
        assert_eq!(part_c["custom_string"].as_str().unwrap(), "hello");
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
    /// `event` is the full tracepoint spec, e.g. "user_events:opentelemetry_traces_L4K1".
    /// Capture runs for `duration_secs`; the caller emits events on another thread
    /// during this window, mirroring the old `perf record` duration.
    ///
    /// This replaces the previous `perf record` + `perf-decode` external-tool
    /// pipeline with a self-contained, in-process consumer (no external tools, no
    /// temp files, no `sudo` shell-outs).
    fn capture_and_decode_events(duration_secs: u64, event: &str) -> std::io::Result<String> {
        let need_permission = "Need permission to access tracefs/perf_events (run via sudo?)";

        // Strip the "user_events:" system prefix to get the tracepoint name,
        // e.g. "user_events:opentelemetry_traces_L4K1" -> "opentelemetry_traces_L4K1".
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

            // Event identity, e.g. "opentelemetry_traces:Span".
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
