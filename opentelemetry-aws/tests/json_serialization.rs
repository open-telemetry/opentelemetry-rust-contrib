//! Integration tests for JSON serialization of segment documents.
//!
//! Tests that segment documents serialize to valid JSON matching the X-Ray
//! segment document schema.

mod common;
// Avoid clippy warning for deadcode
pub use common::*;

use opentelemetry::trace::{SpanId, SpanKind};
use opentelemetry::KeyValue;
use opentelemetry_aws::xray_exporter::{SegmentTranslator, XrayExporter};
use opentelemetry_sdk::trace::SpanExporter;
use opentelemetry_sdk::Resource;

#[tokio::test]
async fn test_segment_json_is_valid() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // MockExporter already provides serde_json::Value - verify it's an object
    assert!(json.is_object(), "Document should be a JSON object");

    // Verify basic required fields exist
    assert_field_exists(json, "name");
    assert_field_exists(json, "id");
    assert_field_exists(json, "trace_id");
    assert_field_exists(json, "start_time");
}

#[tokio::test]
async fn test_segment_has_required_fields() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Required fields for segments with correct types
    assert_field_eq(json, "name", "test-span");
    assert_field_eq(json, "id", "1111111111111111");

    // Verify trace_id is a string
    let trace_id_str = get_nested_value(json, "trace_id")
        .and_then(|v| v.as_str())
        .expect("trace_id should be a string");
    assert!(
        trace_id_str.starts_with("1-"),
        "trace_id should start with '1-'"
    );

    // Verify start_time is a number
    let start_time = get_nested_value(json, "start_time")
        .and_then(|v| v.as_f64())
        .expect("start_time should be a number");
    assert!(start_time > 0.0, "start_time should be positive");
}

#[tokio::test]
async fn test_subsegment_has_required_fields() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let parent_span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());
    let span = create_basic_span(
        "subsegment",
        SpanKind::Client,
        trace_id,
        span_id,
        Some(parent_span_id),
    );

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Required fields for subsegments with correct values and types
    assert_field_eq(json, "type", "subsegment");
    assert_field_eq(json, "name", "subsegment");
    assert_field_eq(json, "id", "2222222222222222");
    assert_field_eq(json, "parent_id", "1111111111111111");

    // Verify start_time is a number
    let start_time = get_nested_value(json, "start_time")
        .and_then(|v| v.as_f64())
        .expect("start_time should be a number");
    assert!(start_time > 0.0, "start_time should be positive");
}

#[tokio::test]
async fn test_trace_id_format_in_json() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    let trace_id_str = get_nested_value(json, "trace_id")
        .and_then(|v| v.as_str())
        .expect("trace_id should be a string");

    // X-Ray trace ID format: 1-{timestamp}-{random}
    assert!(
        trace_id_str.starts_with("1-"),
        "Trace ID should start with '1-'"
    );

    let parts: Vec<&str> = trace_id_str.split('-').collect();
    assert_eq!(
        parts.len(),
        3,
        "Trace ID should have 3 parts separated by '-'"
    );
    assert_eq!(parts[0], "1", "First part should be '1'");

    // Second part should be hex timestamp (8 hex chars)
    assert_eq!(
        parts[1].len(),
        8,
        "Timestamp part should be 8 hex characters"
    );
    assert!(
        parts[1].chars().all(|c| c.is_ascii_hexdigit()),
        "Timestamp should be hex"
    );

    // Third part should be random (24 hex chars)
    assert_eq!(
        parts[2].len(),
        24,
        "Random part should be 24 hex characters"
    );
    assert!(
        parts[2].chars().all(|c| c.is_ascii_hexdigit()),
        "Random part should be hex"
    );
}

#[tokio::test]
async fn test_span_id_format_in_json() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1234567890abcdefu64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Verify the exact span ID value
    assert_field_eq(json, "id", "1234567890abcdef");
}

#[tokio::test]
async fn test_timestamp_format_in_json() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Timestamps should be numbers (floating point seconds since epoch)
    let start_time = get_nested_value(json, "start_time")
        .and_then(|v| v.as_f64())
        .expect("start_time should be a number");

    assert!(start_time > 0.0, "start_time should be positive");

    // Verify end_time exists and is valid
    assert_field_exists(json, "end_time");
    let end_time = get_nested_value(json, "end_time")
        .and_then(|v| v.as_f64())
        .expect("end_time should be a number");

    assert!(end_time >= start_time, "end_time should be >= start_time");
    assert!(end_time > 0.0, "end_time should be positive");
}

#[tokio::test]
async fn test_http_data_structure() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let parent_span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());
    let mut span = create_basic_span(
        "http-span",
        SpanKind::Client,
        trace_id,
        span_id,
        Some(parent_span_id),
    );

    span.attributes = vec![
        KeyValue::new("http.method", "GET"),
        KeyValue::new("http.url", "https://example.com/api"),
        KeyValue::new("http.status_code", opentelemetry::Value::I64(200)),
    ];

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // HTTP data should exist and be properly structured
    get_nested_value(json, "http")
        .and_then(|v| v.as_object())
        .expect("http should be an object");

    // Verify request structure
    get_nested_value(json, "http.request")
        .and_then(|v| v.as_object())
        .expect("http.request should be an object");

    assert_field_eq(json, "http.request.method", "GET");
    assert_field_eq(json, "http.request.url", "https://example.com/api");

    // Verify response structure
    get_nested_value(json, "http.response")
        .and_then(|v| v.as_object())
        .expect("http.response should be an object");

    assert_field_eq(json, "http.response.status", 200);
}

#[tokio::test]
async fn test_aws_data_structure() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("cloud.provider", "aws"),
            KeyValue::new("cloud.platform", "aws_lambda"),
            KeyValue::new("cloud.region", "us-east-1"),
        ])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_lambda_handler_span(trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // AWS data should exist and be properly structured
    assert_field_exists(json, "aws");

    let aws = get_nested_value(json, "aws")
        .and_then(|v| v.as_object())
        .expect("aws should be an object");

    // Verify it contains some Lambda-specific data
    assert!(!aws.is_empty(), "aws object should not be empty");
}

#[tokio::test]
async fn test_annotations_structure() {
    let mock_exporter = MockExporter::new();

    let translator = SegmentTranslator::new().with_indexed_attr("custom.key".to_string());
    let exporter = XrayExporter::new(mock_exporter.clone()).with_translator(translator);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let mut span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    span.attributes = vec![KeyValue::new("custom.key", "value")];

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Annotations should exist and be properly structured
    assert_field_exists(json, "annotations");

    let annotations = get_nested_value(json, "annotations")
        .and_then(|v| v.as_object())
        .expect("annotations should be an object");

    assert_eq!(annotations.len(), 1, "Should have exactly 1 annotation");

    // Verify the actual annotation value (keys are sanitized: . -> _)
    assert_field_eq(json, "annotations.custom_key", "value");
}

#[tokio::test]
async fn test_metadata_structure() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let mut span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    span.attributes = vec![KeyValue::new("custom.metadata", "value")];

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Metadata should exist and be properly structured
    assert_field_exists(json, "metadata");

    let metadata = get_nested_value(json, "metadata")
        .and_then(|v| v.as_object())
        .expect("metadata should be an object");

    assert_eq!(metadata.len(), 1, "Should have exactly 1 metadata item");

    // Verify the actual metadata value (keys are preserved with dots)
    assert_field_eq(json, ["metadata", "custom.metadata"].as_slice(), "value");
}

#[tokio::test]
async fn test_no_null_values_in_json() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Verify by checking that the object doesn't contain any null values
    fn contains_null(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Null => true,
            serde_json::Value::Object(map) => map.values().any(contains_null),
            serde_json::Value::Array(arr) => arr.iter().any(contains_null),
            _ => false,
        }
    }

    assert!(
        !contains_null(json),
        "Document should not contain null values"
    );
}

#[tokio::test]
async fn test_compact_json_no_whitespace() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Serialize to compact JSON string to verify formatting
    let json_string = serde_json::to_string(json).expect("Should serialize to JSON");

    // Compact JSON should not have unnecessary whitespace
    assert!(
        !json_string.contains("\n"),
        "Compact JSON should not have newlines"
    );
    assert!(
        !json_string.contains("  "),
        "Compact JSON should not have double spaces"
    );

    // Verify we can still read required fields
    assert_field_eq(json, "name", "test-span");
    assert_field_eq(json, "id", "1111111111111111");
    assert_field_exists(json, "trace_id");
}
