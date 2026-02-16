//! Integration tests for error handling and exception translation.
//!
//! Tests the mapping of span status, HTTP status codes, and exception events
//! to X-Ray error, fault, and throttle flags.

mod common;
// Avoid clippy warning for deadcode
pub use common::*;

use opentelemetry::trace::{Event as SpanEvent, SpanId, SpanKind, Status};
use opentelemetry::{KeyValue, Value};
use opentelemetry_aws::xray_exporter::XrayExporter;
use opentelemetry_sdk::trace::SpanExporter;
use opentelemetry_sdk::Resource;
use std::time::{Duration, UNIX_EPOCH};

#[tokio::test]
async fn test_http_4xx_maps_to_error() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let parent_span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());
    let mut span = create_basic_span(
        "http-request",
        SpanKind::Client,
        trace_id,
        span_id,
        Some(parent_span_id),
    );

    span.attributes = vec![
        KeyValue::new("http.method", "GET"),
        KeyValue::new("http.status_code", Value::I64(404)),
    ];
    span.status = Status::error("Not Found");

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // 4xx should map to error flag
    assert_field_eq(json, "error", true);
    assert_field_not_exists(json, "fault");
    assert_field_not_exists(json, "throttle");
}

#[tokio::test]
async fn test_http_5xx_maps_to_fault() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let parent_span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());
    let mut span = create_basic_span(
        "http-request",
        SpanKind::Client,
        trace_id,
        span_id,
        Some(parent_span_id),
    );

    span.attributes = vec![
        KeyValue::new("http.method", "POST"),
        KeyValue::new("http.status_code", Value::I64(500)),
    ];
    span.status = Status::error("Internal Server Error");

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // 5xx should map to fault flag
    assert_field_eq(json, "fault", true);
    assert_field_not_exists(json, "error");
    assert_field_not_exists(json, "throttle");
}

#[tokio::test]
async fn test_http_429_maps_to_throttle() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let parent_span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());
    let mut span = create_basic_span(
        "http-request",
        SpanKind::Client,
        trace_id,
        span_id,
        Some(parent_span_id),
    );

    span.attributes = vec![
        KeyValue::new("http.method", "GET"),
        KeyValue::new("http.status_code", Value::I64(429)),
    ];
    span.status = Status::error("Too Many Requests");

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // 429 should map to throttle and error flags
    assert_field_eq(json, "throttle", true);
    assert_field_eq(json, "error", true);
    assert_field_not_exists(json, "fault");
}

#[tokio::test]
async fn test_error_status_without_http_code() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let parent_span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());
    let mut span = create_basic_span(
        "operation",
        SpanKind::Internal,
        trace_id,
        span_id,
        Some(parent_span_id),
    );

    span.status = Status::error("Something went wrong");

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Error status without HTTP code should map to fault
    assert_field_eq(json, "fault", true);
    assert_field_not_exists(json, "error");
    assert_field_not_exists(json, "throttle");
}

#[tokio::test]
async fn test_exception_event_extraction() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    // Set resource with telemetry.sdk.language for stack trace support
    let resource = Resource::builder()
        .with_attributes(vec![KeyValue::new("telemetry.sdk.language", "rust")])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let parent_span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());
    let mut span = create_basic_span(
        "operation",
        SpanKind::Internal,
        trace_id,
        span_id,
        Some(parent_span_id),
    );

    // Add exception event
    let exception_event = SpanEvent::new(
        "exception",
        UNIX_EPOCH + Duration::from_secs(1700000000),
        vec![
            KeyValue::new("exception.type", "std::io::Error"),
            KeyValue::new("exception.message", "File not found"),
            KeyValue::new(
                "exception.stacktrace",
                r#"
                    0: __rustc::rust_begin_unwind
                        at /rustc/254b59607d4417e9dffbc307138ae5c86280fe4c/library/std/src/panicking.rs:689:5
                    1: core::panicking::panic_fmt
                        at /rustc/254b59607d4417e9dffbc307138ae5c86280fe4c/library/core/src/panicking.rs:80:14
                    2: playground::module::some_function
                        at ./src/main.rs:3:9
                    3: playground::main
                        at ./src/main.rs:10:5
                "#,
            ),
        ],
        0,
    );
    span.events.events.push(exception_event);
    span.status = Status::error("Exception occurred");

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Should have fault flag and cause field
    assert_field_eq(json, "fault", true);
    assert_field_exists(json, "cause");

    // Verify cause structure contains exception details
    let cause = get_nested_value(json, "cause").expect("cause should exist");
    assert!(cause.is_object(), "cause should be an object");

    // Verify exception details in cause
    let exceptions = get_nested_value(json, "cause.exceptions")
        .and_then(|v| v.as_array())
        .expect("cause.exceptions should be an array");
    assert_eq!(exceptions.len(), 1);

    let exception = &exceptions[0];

    assert_field_eq(exception, "type", "std::io::Error");
    assert_field_eq(exception, "message", "File not found");
    assert_field_exists(exception, "stack");
}

#[tokio::test]
async fn test_multiple_exception_events() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    let resource = Resource::builder()
        .with_attributes(vec![KeyValue::new("telemetry.sdk.language", "rust")])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let parent_span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());
    let mut span = create_basic_span(
        "operation",
        SpanKind::Internal,
        trace_id,
        span_id,
        Some(parent_span_id),
    );

    // Add multiple exception events
    span.events.events.push(SpanEvent::new(
        "exception",
        UNIX_EPOCH + Duration::from_secs(1700000000),
        vec![
            KeyValue::new("exception.type", "FirstError"),
            KeyValue::new("exception.message", "First error occurred"),
        ],
        0,
    ));

    span.events.events.push(SpanEvent::new(
        "exception",
        UNIX_EPOCH + Duration::from_secs(1700000001),
        vec![
            KeyValue::new("exception.type", "SecondError"),
            KeyValue::new("exception.message", "Second error occurred"),
        ],
        0,
    ));

    span.status = Status::error("Multiple exceptions");

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    assert_field_eq(json, "fault", true);
    assert_field_exists(json, "cause");

    // Verify cause contains multiple exceptions
    let exceptions = get_nested_value(json, "cause.exceptions")
        .and_then(|v| v.as_array())
        .expect("cause.exceptions should be an array");
    assert_eq!(exceptions.len(), 2, "Should have 2 exceptions");

    // Verify first exception
    assert_field_eq(&exceptions[0], "type", "FirstError");
    assert_field_eq(&exceptions[0], "message", "First error occurred");

    // Verify second exception
    assert_field_eq(&exceptions[1], "type", "SecondError");
    assert_field_eq(&exceptions[1], "message", "Second error occurred");
}

#[tokio::test]
async fn test_ok_status_no_error_flags() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let mut span = create_basic_span("operation", SpanKind::Server, trace_id, span_id, None);

    span.attributes = vec![
        KeyValue::new("http.method", "GET"),
        KeyValue::new("http.status_code", Value::I64(200)),
    ];
    span.status = Status::Ok;

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Should not have error, fault, or throttle flags
    assert_field_not_exists(json, "error");
    assert_field_not_exists(json, "fault");
    assert_field_not_exists(json, "throttle");
    assert_field_not_exists(json, "cause");
}

#[tokio::test]
async fn test_unset_status_no_error_flags() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let parent_span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());
    let span = create_basic_span(
        "operation",
        SpanKind::Internal,
        trace_id,
        span_id,
        Some(parent_span_id),
    );

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Unset status should not have error flags
    assert_field_not_exists(json, "error");
    assert_field_not_exists(json, "fault");
    assert_field_not_exists(json, "throttle");
    assert_field_not_exists(json, "cause");
}

#[tokio::test]
async fn test_http_2xx_with_error_status() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let parent_span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());
    let mut span = create_basic_span(
        "operation",
        SpanKind::Client,
        trace_id,
        span_id,
        Some(parent_span_id),
    );

    // HTTP 200 but error status (application-level error)
    span.attributes = vec![
        KeyValue::new("http.method", "POST"),
        KeyValue::new("http.status_code", Value::I64(200)),
    ];
    span.status = Status::error("Application error");

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Should have fault flag (error status takes precedence)
    assert_field_eq(json, "fault", true);
    assert_field_not_exists(json, "error");
    assert_field_not_exists(json, "throttle");
}

#[tokio::test]
async fn test_exception_with_stacktrace() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    let resource = Resource::builder()
        .with_attributes(vec![KeyValue::new("telemetry.sdk.language", "rust")])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let parent_span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());
    let mut span = create_basic_span(
        "operation",
        SpanKind::Internal,
        trace_id,
        span_id,
        Some(parent_span_id),
    );

    let stacktrace = r#"
   0: my_crate::process_request
             at src/main.rs:42
   1: my_crate::handle_connection
             at src/lib.rs:100
   2: tokio::runtime::task::core::Core<T,S>::poll
             at /home/user/.cargo/registry/src/tokio-1.0.0/src/runtime/task/core.rs:184
"#;

    span.events.events.push(SpanEvent::new(
        "exception",
        UNIX_EPOCH + Duration::from_secs(1700000000),
        vec![
            KeyValue::new("exception.type", "MyError"),
            KeyValue::new("exception.message", "Something went wrong"),
            KeyValue::new("exception.stacktrace", stacktrace),
        ],
        0,
    ));
    span.status = Status::error("Exception");

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    assert_field_eq(json, "fault", true);

    let exceptions = get_nested_value(json, "cause.exceptions")
        .and_then(|v| v.as_array())
        .expect("cause.exceptions should be an array");
    assert_eq!(exceptions.len(), 1, "Should have 1 exception");
    let exception = &exceptions[0];

    // Verify stack trace is present and parsed
    assert_field_eq(exception, "type", "MyError");
    assert_field_eq(exception, "message", "Something went wrong");

    let stack = get_nested_value(exception, "stack")
        .and_then(|v| v.as_array())
        .expect("stack should be an array");

    // Verify stacktrace was parsed into frames
    assert!(stack.len() == 3, "Stack should contain 3 frames");
}

#[tokio::test]
async fn test_aws_sdk_error_attributes() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let parent_span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());
    let mut span = create_basic_span(
        "aws-call",
        SpanKind::Client,
        trace_id,
        span_id,
        Some(parent_span_id),
    );

    span.attributes = vec![
        KeyValue::new("rpc.service", "DynamoDB"),
        KeyValue::new("rpc.method", "PutItem"),
        KeyValue::new("rpc.system", "aws-api"),
        KeyValue::new("http.status_code", Value::I64(400)),
    ];
    span.status = Status::error("AWS SDK error");
    span.events.events.push(SpanEvent::new(
        "HTTP request failure",
        UNIX_EPOCH + Duration::from_secs(1700000000),
        vec![
            KeyValue::new("aws.error.code", "ValidationException"),
            KeyValue::new("aws.error.message", "Invalid parameter"),
        ],
        0,
    ));

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // 400 error should map to error flag
    assert_field_eq(json, "error", true);
    assert_field_not_exists(json, "fault");
    assert_field_not_exists(json, "throttle");

    // Verify AWS error details are captured
    assert_field_exists(json, "cause");
}

#[tokio::test]
async fn test_mixed_success_and_error_spans() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();

    // Success span
    let parent_span_id_1 = SpanId::from_bytes(0x9999999999999999u64.to_be_bytes());
    let mut success_span = create_basic_span(
        "success",
        SpanKind::Client,
        trace_id,
        SpanId::from_bytes(0x1111111111111111u64.to_be_bytes()),
        Some(parent_span_id_1),
    );
    success_span.status = Status::Ok;
    success_span.attributes = vec![KeyValue::new("http.status_code", Value::I64(200))];

    // Error span
    let parent_span_id_2 = SpanId::from_bytes(0x8888888888888888u64.to_be_bytes());
    let mut error_span = create_basic_span(
        "error",
        SpanKind::Client,
        trace_id,
        SpanId::from_bytes(0x2222222222222222u64.to_be_bytes()),
        Some(parent_span_id_2),
    );
    error_span.status = Status::error("Failed");
    error_span.attributes = vec![KeyValue::new("http.status_code", Value::I64(500))];

    exporter
        .export(vec![success_span, error_span])
        .await
        .unwrap();

    let documents = mock_exporter.get_documents();
    dbg!(&documents);
    assert_eq!(documents.len(), 2, "Should export two documents");

    // Success span should not have error flags
    let success_json = &documents[0];
    assert_field_eq(success_json, "name", "success");
    assert_field_not_exists(success_json, "fault");
    assert_field_not_exists(success_json, "error");
    assert_field_not_exists(success_json, "throttle");

    // Error span should have fault flag
    let error_json = &documents[1];
    assert_field_eq(error_json, "name", "error");
    assert_field_eq(error_json, "fault", true);
    assert_field_not_exists(error_json, "error");
    assert_field_not_exists(error_json, "throttle");
}
