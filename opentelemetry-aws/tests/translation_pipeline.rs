//! Integration tests for the complete translation pipeline.
//!
//! Tests the end-to-end workflow of translating OpenTelemetry spans into
//! X-Ray segment documents and exporting them.

mod common;
// Avoid clippy warning for deadcode
pub use common::*;

use opentelemetry::trace::{SpanId, SpanKind};
use opentelemetry::KeyValue;
use opentelemetry_aws::xray_exporter::{SegmentTranslator, XrayExporter};
use opentelemetry_sdk::trace::SpanExporter;
use opentelemetry_sdk::Resource;

#[tokio::test]
async fn test_translate_and_export_lambda_handler_span() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    // Set up Lambda resource
    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("service.name", "my-lambda-function"),
            KeyValue::new("cloud.provider", "aws"),
            KeyValue::new("cloud.platform", "aws_lambda"),
            KeyValue::new("faas.name", "my-lambda-function"),
            KeyValue::new("cloud.region", "us-east-1"),
        ])
        .build();
    exporter.set_resource(&resource);

    // Create a Lambda handler span
    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1234567890abcdefu64.to_be_bytes());
    let span = create_lambda_handler_span(trace_id, span_id, None);

    // Export the span
    exporter.export(vec![span]).await.unwrap();

    // Verify a document was created
    let documents = mock_exporter.get_documents();
    assert_eq!(documents.len(), 1, "Should export one document");

    // Verify the document contains expected fields
    let json = &documents[0];
    assert_field_eq(json, "name", "my-lambda-function");
    assert_field_eq(json, "id", "1234567890abcdef");
    assert_field_exists(json, "trace_id");
    assert_field_exists(json, "start_time");
    assert_field_exists(json, "end_time");
    assert_field_eq(json, "origin", "AWS::Lambda::Function");
}

#[tokio::test]
async fn test_translate_and_export_dynamodb_span() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("service.name", "my-service"),
            KeyValue::new("cloud.provider", "aws"),
        ])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();
    let parent_span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());
    let span = create_dynamodb_span(trace_id, span_id, parent_span_id);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    assert_eq!(documents.len(), 1, "Should export one document");

    let json = &documents[0];
    assert_field_eq(json, "type", "subsegment");
    assert_field_eq(json, "name", "DynamoDB");
    assert_field_eq(json, "id", "2222222222222222");
    assert_field_exists(json, "start_time");
    assert_field_exists(json, "end_time");
    assert_field_eq(json, "parent_id", "1111111111111111");
    assert_field_eq(json, "namespace", "aws");
    assert_field_eq(json, "aws.region", "us-east-1");
    assert_field_eq(json, "aws.operation", "PutItem");
    assert_field_eq(json, "aws.table_name", "my-table");
}

#[tokio::test]
async fn test_translate_and_export_http_client_span() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();
    let parent_span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span_id = SpanId::from_bytes(0x3333333333333333u64.to_be_bytes());
    let span = create_http_client_span(trace_id, span_id, parent_span_id, 200);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    assert_eq!(documents.len(), 1, "Should export one document");

    let json = &documents[0];
    assert_field_eq(json, "type", "subsegment");
    assert_field_eq(json, "name", "GET /users/:id");
    assert_field_eq(json, "id", "3333333333333333");
    assert_field_eq(json, "parent_id", "1111111111111111");

    // Verify HTTP data structure
    assert_field_exists(json, "http");
    assert_field_eq(json, "http.request.method", "GET");
    assert_field_eq(json, "http.response.status", 200);
}

#[tokio::test]
async fn test_translate_batch_of_spans() {
    let mock_exporter = MockExporter::new();
    let exporter = XrayExporter::new(mock_exporter.clone());

    let trace_id = create_valid_trace_id();

    // Create multiple spans
    let spans = vec![
        create_basic_span(
            "span1",
            SpanKind::Server,
            trace_id,
            SpanId::from_bytes(0x1111111111111111u64.to_be_bytes()),
            None,
        ),
        create_basic_span(
            "span2",
            SpanKind::Client,
            trace_id,
            SpanId::from_bytes(0x2222222222222222u64.to_be_bytes()),
            Some(SpanId::from_bytes(0x3333333333333333u64.to_be_bytes())),
        ),
        create_basic_span(
            "span3",
            SpanKind::Internal,
            trace_id,
            SpanId::from_bytes(0x4444444444444444u64.to_be_bytes()),
            Some(SpanId::from_bytes(0x5555555555555555u64.to_be_bytes())),
        ),
    ];

    exporter.export(spans).await.unwrap();

    let documents = mock_exporter.get_documents();
    assert_eq!(documents.len(), 3, "Should export three documents");

    // Verify each document
    let span1_json = &documents[0];
    assert_field_eq(span1_json, "name", "span1");
    assert_field_eq(span1_json, "id", "1111111111111111");
    assert_field_exists(span1_json, "trace_id");

    let span2_json = &documents[1];
    assert_field_eq(span2_json, "name", "span2");
    assert_field_eq(span2_json, "id", "2222222222222222");
    assert_field_eq(span2_json, "parent_id", "3333333333333333");

    let span3_json = &documents[2];
    assert_field_eq(span3_json, "name", "span3");
    assert_field_eq(span3_json, "id", "4444444444444444");
    assert_field_eq(span3_json, "parent_id", "5555555555555555");
}

#[tokio::test]
async fn test_translate_with_custom_translator() {
    let mock_exporter = MockExporter::new();

    // Create translator with indexed attributes
    let translator = SegmentTranslator::new()
        .with_indexed_attr("http.method".to_string())
        .with_indexed_attr("http.status_code".to_string());

    let exporter = XrayExporter::new(mock_exporter.clone()).with_translator(translator);

    let trace_id = create_valid_trace_id();
    let parent_span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());
    let span = create_http_client_span(trace_id, span_id, parent_span_id, 200);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Indexed attributes should appear in annotations
    assert_field_exists(json, "annotations");
    assert_field_eq(json, "annotations.http_method", "GET");
    assert_field_eq(json, "annotations.http_status_code", 200);
}

#[tokio::test]
async fn test_translate_with_index_all_attributes() {
    let mock_exporter = MockExporter::new();

    let translator = SegmentTranslator::new().index_all_attrs();
    let exporter = XrayExporter::new(mock_exporter.clone()).with_translator(translator);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let mut span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    // Add custom attributes
    span.attributes = vec![
        KeyValue::new("custom.attr1", "value1"),
        KeyValue::new("custom.attr2", "value2"),
    ];

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // With index_all_attrs, custom attributes should be in annotations
    assert_field_exists(json, "annotations");
    assert_field_eq(json, "annotations.custom_attr1", "value1");
    assert_field_eq(json, "annotations.custom_attr2", "value2");
}

#[tokio::test]
async fn test_translate_realistic_lambda_workflow() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    // Set up realistic Lambda resource
    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("service.name", "order-processor"),
            KeyValue::new("cloud.provider", "aws"),
            KeyValue::new("cloud.platform", "aws_lambda"),
            KeyValue::new("faas.name", "order-processor"),
            KeyValue::new("faas.version", "1"),
            KeyValue::new("cloud.region", "us-east-1"),
            KeyValue::new("telemetry.sdk.name", "opentelemetry"),
            KeyValue::new("telemetry.sdk.language", "rust"),
            KeyValue::new("telemetry.sdk.version", "0.31.0"),
        ])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();

    // Lambda handler span
    let handler_span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let handler_span = create_lambda_handler_span(trace_id, handler_span_id, None);

    // DynamoDB call within the handler
    let dynamo_span_id = SpanId::from_bytes(0x2222222222222222u64.to_be_bytes());
    let dynamo_span = create_dynamodb_span(trace_id, dynamo_span_id, handler_span_id);

    // HTTP call to external API
    let http_span_id = SpanId::from_bytes(0x3333333333333333u64.to_be_bytes());
    let http_span = create_http_client_span(trace_id, http_span_id, handler_span_id, 200);

    exporter
        .export(vec![handler_span, dynamo_span, http_span])
        .await
        .unwrap();

    let documents = mock_exporter.get_documents();
    assert_eq!(documents.len(), 3, "Should export three documents");

    // Verify the handler span is a segment (has trace_id, no type field)
    let handler_json = &documents[0];
    assert_field_eq(handler_json, "name", "order-processor");
    assert_field_eq(handler_json, "id", "1111111111111111");
    assert_field_exists(handler_json, "trace_id");
    assert_field_eq(handler_json, "origin", "AWS::Lambda::Function");

    // Verify DynamoDB subsegment has parent_id
    let dynamo_json = &documents[1];
    assert_field_eq(dynamo_json, "name", "DynamoDB");
    assert_field_eq(dynamo_json, "type", "subsegment");
    assert_field_eq(dynamo_json, "id", "2222222222222222");
    assert_field_eq(dynamo_json, "parent_id", "1111111111111111");
    assert_field_eq(dynamo_json, "namespace", "aws");

    // Verify HTTP subsegment has parent_id
    let http_json = &documents[2];
    assert_field_eq(http_json, "name", "GET /users/:id");
    assert_field_eq(http_json, "type", "subsegment");
    assert_field_eq(http_json, "id", "3333333333333333");
    assert_field_eq(http_json, "parent_id", "1111111111111111");
    assert_field_exists(http_json, "http");
}
