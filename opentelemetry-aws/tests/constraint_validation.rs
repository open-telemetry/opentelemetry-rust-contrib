//! Integration tests for X-Ray constraint validation.
//!
//! Tests that the translator properly validates and handles X-Ray constraints
//! such as name regex, string lengths, timestamp bounds, and required fields.

mod common;
// Avoid clippy warning for deadcode
pub use common::*;

use core::f64::consts::PI;
use opentelemetry::trace::{SpanId, SpanKind, TraceId};
use opentelemetry::{KeyValue, Value};
use opentelemetry_aws::xray_exporter::{SegmentTranslator, XrayExporter};
use opentelemetry_sdk::trace::SpanExporter;
use std::time::{SystemTime, UNIX_EPOCH};

#[tokio::test]
async fn test_valid_segment_name() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span(
        "valid-segment-name",
        SpanKind::Server,
        trace_id,
        span_id,
        None,
    );

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    assert_eq!(documents.len(), 1, "Should export one document");

    let json = &documents[0];
    assert_field_eq(json, "name", "valid-segment-name");
    assert_field_eq(json, "id", "1111111111111111");
    assert_field_exists(json, "trace_id");
    assert_field_exists(json, "start_time");
    assert_field_exists(json, "end_time");
}

#[tokio::test]
async fn test_long_segment_name_truncation() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());

    // Create a name longer than 200 characters (X-Ray limit)
    let long_name = "a".repeat(250);
    let span = create_basic_span(long_name.leak(), SpanKind::Server, trace_id, span_id, None);

    let result = exporter.export(vec![span]).await;

    if result.is_ok() {
        let documents = mock_exporter.get_documents();
        if !documents.is_empty() {
            let json = &documents[0];
            // If export succeeded, verify the name field exists and is not too long
            let name = get_nested_value(json, "name")
                .and_then(|v| v.as_str())
                .expect("name should be a string");
            assert!(
                name.len() <= 200,
                "Name should be truncated to 200 characters or less, got {} chars",
                name.len()
            );
            // Verify other required fields are present
            assert_field_exists(json, "id");
            assert_field_exists(json, "start_time");
        }
    }
}

#[tokio::test]
async fn test_valid_trace_id_timestamp() {
    let mock_exporter = MockExporter::new();
    let translator = SegmentTranslator::new();
    let exporter = XrayExporter::new(mock_exporter.clone()).with_translator(translator);

    // Create a trace ID with current timestamp
    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Verify trace_id format: 1-{hex8}-{hex24}
    let trace_id_str = get_nested_value(json, "trace_id")
        .and_then(|v| v.as_str())
        .expect("trace_id should be a string");

    assert!(
        trace_id_str.starts_with("1-"),
        "Trace ID should start with '1-'"
    );

    let parts: Vec<&str> = trace_id_str.split('-').collect();
    assert_eq!(parts.len(), 3, "Trace ID should have 3 parts");
    assert_eq!(parts[0], "1");
    assert_eq!(parts[1].len(), 8, "Timestamp part should be 8 hex chars");
    assert_eq!(parts[2].len(), 24, "Random part should be 24 hex chars");
}

#[tokio::test]
async fn test_skip_timestamp_validation() {
    let mock_exporter = MockExporter::new();

    // Enable skip_timestamp_validation for testing with old timestamps
    let translator = SegmentTranslator::new().skip_timestamp_validation();
    let exporter = XrayExporter::new(mock_exporter.clone()).with_translator(translator);

    // Create a trace ID with a very old timestamp (more than 30 days ago)
    let old_timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_sub(40 * 24 * 60 * 60); // 40 days ago

    let trace_id_u128 = ((old_timestamp as u128) << 96) | 0x89abcdef0123456789abcdef;
    let trace_id = TraceId::from_bytes(trace_id_u128.to_be_bytes());

    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Verify the trace_id contains the old timestamp
    let trace_id_str = get_nested_value(json, "trace_id")
        .and_then(|v| v.as_str())
        .expect("trace_id should be a string");

    let timestamp_hex = trace_id_str.split('-').nth(1).unwrap();
    let parsed_timestamp = u32::from_str_radix(timestamp_hex, 16).expect("Should parse timestamp");

    assert_eq!(
        parsed_timestamp as u64, old_timestamp,
        "Trace ID should contain the old timestamp"
    );

    // Verify document structure is still valid
    assert_field_eq(json, "name", "test-span");
    assert_field_exists(json, "id");
    assert_field_exists(json, "start_time");
    assert_field_exists(json, "end_time");
}

#[tokio::test]
async fn test_annotation_key_constraints() {
    let mock_exporter = MockExporter::new();

    let translator = SegmentTranslator::new().index_all_attrs();
    let exporter = XrayExporter::new(mock_exporter.clone()).with_translator(translator);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let mut span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    // Test various annotation key formats
    span.attributes = vec![
        KeyValue::new("valid_key", "value1"),
        KeyValue::new("valid.key.with.dots", "value2"),
        KeyValue::new("valid-key-with-dashes", "value3"),
        KeyValue::new("key123", "value4"),
    ];

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Verify all annotations are present with sanitized keys (dots -> underscores, dashes -> underscores)
    assert_field_eq(json, "annotations.valid_key", "value1");
    assert_field_eq(json, "annotations.valid_key_with_dots", "value2");
    assert_field_eq(json, "annotations.valid_key_with_dashes", "value3");
    assert_field_eq(json, "annotations.key123", "value4");

    // Verify annotations is an object with correct count
    let annotations = get_nested_value(json, "annotations")
        .and_then(|v| v.as_object())
        .expect("annotations should be an object");
    assert_eq!(annotations.len(), 4, "Should have exactly 4 annotations");
}

#[tokio::test]
async fn test_annotation_value_types() {
    let mock_exporter = MockExporter::new();

    let translator = SegmentTranslator::new().index_all_attrs();
    let exporter = XrayExporter::new(mock_exporter.clone()).with_translator(translator);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let mut span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    // X-Ray annotations support: string, number, boolean
    span.attributes = vec![
        KeyValue::new("string_annotation", "test"),
        KeyValue::new("int_annotation", Value::I64(42)),
        KeyValue::new("float_annotation", Value::F64(PI)),
        KeyValue::new("bool_annotation", Value::Bool(true)),
    ];

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Verify each annotation type is preserved correctly
    assert_field_eq(json, "annotations.string_annotation", "test");
    assert_field_eq(json, "annotations.int_annotation", 42);
    assert_field_eq(json, "annotations.float_annotation", PI);
    assert_field_eq(json, "annotations.bool_annotation", true);

    // Verify annotations object has correct count
    let annotations = get_nested_value(json, "annotations")
        .and_then(|v| v.as_object())
        .expect("annotations should be an object");
    assert_eq!(annotations.len(), 4, "Should have exactly 4 annotations");
}

#[tokio::test]
async fn test_max_annotations_limit() {
    let mock_exporter = MockExporter::new();

    let translator = SegmentTranslator::new().index_all_attrs();
    let exporter = XrayExporter::new(mock_exporter.clone()).with_translator(translator);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let mut span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    // Create more than 50 attributes (X-Ray annotation limit)
    let mut attributes = Vec::new();
    for i in 0..60 {
        attributes.push(KeyValue::new(format!("attr_{i}"), format!("value_{i}")));
    }
    span.attributes = attributes;

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // X-Ray has a limit of 50 annotations
    let annotations = get_nested_value(json, "annotations")
        .and_then(|v| v.as_object())
        .expect("annotations should be an object");
    assert_eq!(
        annotations.len(),
        50,
        "Should have exactly 50 annotations (the limit)"
    );

    // The remaining 10 should be in metadata
    let metadata = get_nested_value(json, "metadata")
        .and_then(|v| v.as_object())
        .expect("metadata should be an object");
    assert!(
        metadata.len() >= 10,
        "Should have at least 10 items in metadata (overflow)"
    );

    // Verify a few specific annotations (first ones should be in annotations)
    assert_field_eq(json, "annotations.attr_0", "value_0");
    assert_field_eq(json, "annotations.attr_1", "value_1");
    assert_field_eq(json, "annotations.attr_49", "value_49");
}

#[tokio::test]
async fn test_string_length_constraints() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let mut span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    let long_value = "x".repeat(1000);
    // Test with very long string values
    span.attributes = vec![
        KeyValue::new("long_string", long_value.clone()),
        KeyValue::new("normal_string", "normal value"),
    ];

    let result = exporter.export(vec![span]).await;

    if result.is_ok() {
        let documents = mock_exporter.get_documents();
        let json = &documents[0];

        // Long strings should be in metadata (not annotations due to 1000 char limit)
        assert_field_eq(json, "metadata.long_string", &long_value);
        assert_field_eq(json, "metadata.normal_string", "normal value");

        // Verify metadata exists
        let metadata = get_nested_value(json, "metadata")
            .and_then(|v| v.as_object())
            .expect("metadata should be an object");
        assert_eq!(metadata.len(), 2, "Should have 2 items in metadata");
    }
}

#[tokio::test]
async fn test_required_fields_present() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Verify required fields with correct types
    assert_field_eq(json, "name", "test-span");
    assert_field_eq(json, "id", "1111111111111111");

    // Verify trace_id format
    let trace_id_str = get_nested_value(json, "trace_id")
        .and_then(|v| v.as_str())
        .expect("trace_id should be a string");
    assert!(
        trace_id_str.starts_with("1-"),
        "trace_id should start with '1-'"
    );

    // Verify timestamps are numbers
    let start_time = get_nested_value(json, "start_time")
        .and_then(|v| v.as_f64())
        .expect("start_time should be a number");
    assert!(start_time > 0.0, "start_time should be positive");

    let end_time = get_nested_value(json, "end_time")
        .and_then(|v| v.as_f64())
        .expect("end_time should be a number");
    assert!(end_time >= start_time, "end_time should be >= start_time");
}

#[tokio::test]
async fn test_subsegment_required_fields() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let parent_span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());

    // Client span becomes a subsegment
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

    // Verify subsegment-specific fields
    assert_field_eq(json, "type", "subsegment");
    assert_field_eq(json, "name", "subsegment");
    assert_field_eq(json, "id", "2222222222222222");
    assert_field_eq(json, "parent_id", "1111111111111111");

    // Subsegments should have trace_id
    assert_field_exists(json, "trace_id");

    // Verify timestamps
    let start_time = get_nested_value(json, "start_time")
        .and_then(|v| v.as_f64())
        .expect("start_time should be a number");
    assert!(start_time > 0.0);

    let end_time = get_nested_value(json, "end_time")
        .and_then(|v| v.as_f64())
        .expect("end_time should be a number");
    assert!(end_time >= start_time);

    // Subsegments should NOT have origin field
    assert_field_not_exists(json, "origin");
}

#[tokio::test]
async fn test_trace_id_format() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // X-Ray trace ID format: 1-{hex8}-{hex24}
    let trace_id_str = get_nested_value(json, "trace_id")
        .and_then(|v| v.as_str())
        .expect("trace_id should be a string");

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
async fn test_span_id_format() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1234567890abcdefu64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Span ID should be a 16-character hex string
    assert_field_eq(json, "id", "1234567890abcdef");

    let id_str = get_nested_value(json, "id")
        .and_then(|v| v.as_str())
        .expect("id should be a string");

    assert_eq!(id_str.len(), 16, "Span ID should be 16 hex characters");
    assert!(
        id_str.chars().all(|c| c.is_ascii_hexdigit()),
        "Span ID should be hex"
    );
}

#[tokio::test]
async fn test_empty_span_batch() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    exporter.export(vec![]).await.unwrap();

    let documents = mock_exporter.get_documents();
    assert_eq!(documents.len(), 0, "Should not export any documents");
}

#[tokio::test]
async fn test_special_characters_in_attributes() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let mut span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    // Test special characters that need JSON escaping
    span.attributes = vec![
        KeyValue::new("quote_attr", "value with \"quotes\""),
        KeyValue::new("newline_attr", "value with\nnewline"),
        KeyValue::new("backslash_attr", "value with \\ backslash"),
        KeyValue::new("unicode_attr", "value with émojis 🚀"),
    ];

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Verify each attribute with special characters is preserved correctly
    assert_field_eq(json, "metadata.quote_attr", "value with \"quotes\"");
    assert_field_eq(json, "metadata.newline_attr", "value with\nnewline");
    assert_field_eq(json, "metadata.backslash_attr", "value with \\ backslash");
    assert_field_eq(json, "metadata.unicode_attr", "value with émojis 🚀");

    // Verify metadata object has correct count
    let metadata = get_nested_value(json, "metadata")
        .and_then(|v| v.as_object())
        .expect("metadata should be an object");
    assert_eq!(metadata.len(), 4, "Should have exactly 4 items in metadata");
}
