//! Integration tests for attribute processing.
//!
//! Tests the translation of span attributes to X-Ray annotations and metadata,
//! including HTTP, AWS, SQL, and custom attributes.

mod common;
// Avoid clippy warning for deadcode
pub use common::*;

use opentelemetry::trace::{SpanId, SpanKind};
use opentelemetry::{KeyValue, Value};
use opentelemetry_aws::xray_exporter::{SegmentTranslator, XrayExporter};
use opentelemetry_sdk::trace::SpanExporter;

#[tokio::test]
async fn test_http_attributes_translation() {
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
        KeyValue::new("http.url", "https://api.example.com/v1/users"),
        KeyValue::new("http.status_code", Value::I64(201)),
        KeyValue::new("net.peer.name", "api.example.com"),
        KeyValue::new("net.peer.port", Value::I64(443)),
    ];

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Verify HTTP data is present
    assert_field_exists(json, "http");

    // Verify request data
    assert_field_eq(json, "http.request.method", "POST");
    assert_field_exists(json, "http.request.url");

    // Verify response data
    assert_field_eq(json, "http.response.status", 201);
}

#[tokio::test]
async fn test_aws_attributes_translation() {
    let mock_exporter = MockExporter::new();

    // Index attributes
    let translator = SegmentTranslator::new()
        .with_indexed_attr("rpc.service".to_string())
        .with_indexed_attr("rpc.method".to_string());

    let exporter = XrayExporter::new(mock_exporter.clone()).with_translator(translator);

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
        KeyValue::new("rpc.method", "GetItem"),
        KeyValue::new("rpc.system", "aws-api"),
        KeyValue::new("aws.request_id", "ABCD1234EFGH5678"),
        KeyValue::new("aws.region", "us-west-2"),
        KeyValue::new(
            "aws.dynamodb.table_names",
            Value::Array(opentelemetry::Array::String(vec![
                opentelemetry::StringValue::from("Users"),
            ])),
        ),
    ];

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Verify AWS data is present
    assert_field_exists(json, "aws");
    assert_field_eq(json, "namespace", "aws");

    // Attributes should ALSO be indexed as annotations
    assert_field_eq(json, "annotations.rpc_service", "DynamoDB");
    assert_field_eq(json, "annotations.rpc_method", "GetItem");
}

#[tokio::test]
async fn test_sql_attributes_translation() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let parent_span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());
    let mut span = create_basic_span(
        "db-query",
        SpanKind::Client,
        trace_id,
        span_id,
        Some(parent_span_id),
    );

    span.attributes = vec![
        KeyValue::new("db.system", "postgresql"),
        KeyValue::new("db.name", "mydb"),
        KeyValue::new("db.statement", "SELECT * FROM users WHERE id = $1"),
        KeyValue::new("db.user", "app_user"),
        KeyValue::new("net.peer.name", "db.example.com"),
        KeyValue::new("net.peer.port", Value::I64(5432)),
    ];

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Verify SQL data is present
    assert_field_exists(json, "sql");

    // Verify some SQL fields are populated
    assert_field_eq(json, "sql.database_type", "postgresql");
    assert_field_exists(json, "sql.sanitized_query");
}

#[tokio::test]
async fn test_indexed_attributes_as_annotations() {
    let mock_exporter = MockExporter::new();

    let translator = SegmentTranslator::new()
        .with_indexed_attr("custom.user_id".to_string())
        .with_indexed_attr("custom.request_type".to_string());

    let exporter = XrayExporter::new(mock_exporter.clone()).with_translator(translator);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let mut span = create_basic_span("custom-span", SpanKind::Server, trace_id, span_id, None);

    span.attributes = vec![
        KeyValue::new("custom.user_id", "user123"),
        KeyValue::new("custom.request_type", "payment"),
        KeyValue::new("custom.other_data", "not_indexed"),
    ];

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Indexed attributes should be in annotations
    // (sanitized keys)
    assert_field_eq(
        json,
        ["annotations", "custom_user_id"].as_slice(),
        "user123",
    );
    assert_field_eq(
        json,
        ["annotations", "custom_request_type"].as_slice(),
        "payment",
    );

    // Non-Indexed attributes should NOT be in annotations
    assert_field_not_exists(json, ["annotations", "custom_other_data"].as_slice());

    // Non-indexed attributes should be in metadata
    assert_field_eq(
        json,
        ["metadata", "custom.other_data"].as_slice(),
        "not_indexed",
    );
}

#[tokio::test]
async fn test_index_all_attributes() {
    let mock_exporter = MockExporter::new();

    let translator = SegmentTranslator::new().index_all_attrs();
    let exporter = XrayExporter::new(mock_exporter.clone()).with_translator(translator);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let mut span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    span.attributes = vec![
        KeyValue::new("attr1", "value1"),
        KeyValue::new("attr2", Value::I64(42)),
        KeyValue::new("attr3", Value::Bool(true)),
    ];

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // All attributes should be indexed as annotations
    assert_field_eq(json, "annotations.attr1", "value1");
    assert_field_eq(json, "annotations.attr2", 42);
    assert_field_eq(json, "annotations.attr3", true);
}

#[tokio::test]
async fn test_attribute_types_conversion() {
    let mock_exporter = MockExporter::new();

    let translator = SegmentTranslator::new().index_all_attrs();
    let exporter = XrayExporter::new(mock_exporter.clone()).with_translator(translator);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let mut span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    span.attributes = vec![
        KeyValue::new("string_attr", "hello"),
        KeyValue::new("int_attr", Value::I64(123)),
        KeyValue::new("float_attr", Value::F64(45.67)),
        KeyValue::new("bool_attr", Value::Bool(true)),
        KeyValue::new(
            "array_attr",
            Value::Array(opentelemetry::Array::String(vec![
                opentelemetry::StringValue::from("item1"),
                opentelemetry::StringValue::from("item2"),
            ])),
        ),
    ];

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Verify different types are handled in annotations
    assert_field_eq(json, "annotations.string_attr", "hello");
    assert_field_eq(json, "annotations.int_attr", 123);
    assert_field_eq(json, "annotations.float_attr", 45.67);
    assert_field_eq(json, "annotations.bool_attr", true);
}

#[tokio::test]
async fn test_aws_xray_annotations_attribute() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let mut span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    // The aws.xray.annotations attribute allows specifying which attributes to index
    span.attributes = vec![
        KeyValue::new("custom.field1", "value1"),
        KeyValue::new("custom.field2", "value2"),
        KeyValue::new(
            "aws.xray.annotations",
            Value::Array(opentelemetry::Array::String(vec![
                opentelemetry::StringValue::from("custom.field1"),
            ])),
        ),
    ];

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // field1 should be in annotations, field2 in metadata
    assert_field_eq(json, "annotations.custom_field1", "value1");

    // field2 should be in metadata since it was not listed in aws.xray.annotations
    assert_field_eq(json, ["metadata", "custom.field2"].as_slice(), "value2");
}

#[tokio::test]
async fn test_resource_attributes_translation() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    // Set resource with EC2 metadata
    use opentelemetry_sdk::Resource;
    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("cloud.provider", "aws"),
            KeyValue::new("cloud.platform", "aws_ec2"),
            KeyValue::new("cloud.region", "us-east-1"),
            KeyValue::new("cloud.availability_zone", "us-east-1a"),
            KeyValue::new("host.id", "i-1234567890abcdef0"),
            KeyValue::new("host.type", "t3.medium"),
        ])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // EC2 metadata should be in the aws field
    assert_field_eq(json, "origin", "AWS::EC2::Instance");
}

#[tokio::test]
async fn test_ecs_resource_attributes() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    use opentelemetry_sdk::Resource;
    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("cloud.provider", "aws"),
            KeyValue::new("cloud.platform", "aws_ecs"),
            KeyValue::new("cloud.region", "us-west-2"),
            KeyValue::new(
                "aws.ecs.cluster.arn",
                "arn:aws:ecs:us-west-2:123456789012:cluster/my-cluster",
            ),
            KeyValue::new(
                "aws.ecs.task.arn",
                "arn:aws:ecs:us-west-2:123456789012:task/my-task",
            ),
            KeyValue::new("container.id", "abc123"),
        ])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    assert_field_eq(json, "origin", "AWS::ECS::Container");
}

#[tokio::test]
async fn test_lambda_resource_attributes() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    use opentelemetry_sdk::Resource;
    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("cloud.provider", "aws"),
            KeyValue::new("cloud.platform", "aws_lambda"),
            KeyValue::new("faas.name", "my-function"),
            KeyValue::new("faas.version", "1"),
            KeyValue::new("cloud.region", "eu-west-1"),
        ])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    assert_field_eq(json, "origin", "AWS::Lambda::Function");
}

#[tokio::test]
async fn test_many_attributes_annotation_limit() {
    let mock_exporter = MockExporter::new();

    // Index all attributes - X-Ray has a limit of 50 annotations
    let translator = SegmentTranslator::new().index_all_attrs();
    let exporter = XrayExporter::new(mock_exporter.clone()).with_translator(translator);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let mut span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    // Create more than 50 attributes
    let mut attributes = Vec::new();
    for i in 0..60 {
        attributes.push(KeyValue::new(format!("attr_{i}"), format!("value_{i}")));
    }
    span.attributes = attributes;

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Should have both annotations and metadata
    // Excess attributes beyond 50 should go to metadata
    assert_field_exists(json, "annotations");
    assert_field_exists(json, "metadata");

    let annotations = get_nested_value(json, "annotations")
        .and_then(|v| v.as_object())
        .expect("annotations should be an object");
    let metadata = get_nested_value(json, "metadata")
        .and_then(|v| v.as_object())
        .expect("metadata should be an object");

    // X-Ray has a limit of 50 annotations
    assert_eq!(
        annotations.len(),
        50,
        "Should have exactly 50 annotations (the limit)"
    );

    // The remaining 10 should be in metadata
    assert!(
        metadata.len() >= 10,
        "Should have at least 10 items in metadata (overflow)"
    );
}

#[tokio::test]
async fn test_subsegment_preserves_timing_and_parent() {
    use std::time::{Duration, UNIX_EPOCH};

    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x3333333333333333u64.to_be_bytes());
    let parent_span_id = SpanId::from_bytes(0x4444444444444444u64.to_be_bytes());

    // Use SpanKind::Client to create a subsegment
    let mut span = create_basic_span(
        "test-subsegment",
        SpanKind::Client,
        trace_id,
        span_id,
        Some(parent_span_id),
    );

    // Set specific timing values to verify they're preserved
    let start_timestamp = UNIX_EPOCH + Duration::from_millis(1234567890123);
    let end_timestamp = UNIX_EPOCH + Duration::from_millis(1234567890456);
    span.start_time = start_timestamp;
    span.end_time = end_timestamp;

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Verify it's a subsegment
    assert_field_eq(json, "type", "subsegment");

    // Verify parent_id is preserved correctly
    assert_field_eq(json, "parent_id", "4444444444444444");

    // Verify span_id is preserved correctly
    assert_field_eq(json, "id", "3333333333333333");

    // Verify start_time is preserved (converted to seconds as f64)
    let expected_start = start_timestamp
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    assert_field_eq(json, "start_time", expected_start);

    // Verify end_time is preserved (converted to seconds as f64)
    let expected_end = end_timestamp
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    assert_field_eq(json, "end_time", expected_end);

    // Verify trace_id is present
    assert_field_exists(json, "trace_id");

    // Verify name is preserved
    assert_field_eq(json, "name", "test-subsegment");
}
