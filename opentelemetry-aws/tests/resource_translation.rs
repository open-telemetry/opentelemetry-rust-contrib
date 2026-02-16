//! Integration tests for resource attribute translation.
//!
//! Tests the extraction of AWS-specific metadata from resource attributes
//! for EC2, ECS, EKS, Lambda, and Elastic Beanstalk environments.

mod common;
// Avoid clippy warning for deadcode
pub use common::*;

use opentelemetry::trace::{SpanId, SpanKind};
use opentelemetry::KeyValue;
use opentelemetry_aws::xray_exporter::XrayExporter;
use opentelemetry_sdk::trace::SpanExporter;
use opentelemetry_sdk::Resource;

#[tokio::test]
async fn test_ec2_resource_metadata() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("cloud.provider", "aws"),
            KeyValue::new("cloud.platform", "aws_ec2"),
            KeyValue::new("cloud.region", "us-east-1"),
            KeyValue::new("cloud.availability_zone", "us-east-1a"),
            KeyValue::new("cloud.account.id", "123456789012"),
            KeyValue::new("host.id", "i-1234567890abcdef0"),
            KeyValue::new("host.type", "t3.medium"),
            KeyValue::new("host.image.id", "ami-0abcdef1234567890"),
        ])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Verify EC2 origin
    assert_field_eq(json, "origin", "AWS::EC2::Instance");

    // Verify AWS metadata exists and contains EC2-specific data
    assert_field_exists(json, "aws");
    let aws = get_nested_value(json, "aws")
        .and_then(|v| v.as_object())
        .expect("aws should be an object");

    assert!(!aws.is_empty(), "aws object should contain EC2 metadata");

    // Verify some EC2 fields are present
    assert_field_exists(json, "aws.ec2");
}

#[tokio::test]
async fn test_ecs_resource_metadata() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("cloud.provider", "aws"),
            KeyValue::new("cloud.platform", "aws_ecs"),
            KeyValue::new("cloud.region", "us-west-2"),
            KeyValue::new("cloud.account.id", "123456789012"),
            KeyValue::new(
                "aws.ecs.cluster.arn",
                "arn:aws:ecs:us-west-2:123456789012:cluster/my-cluster",
            ),
            KeyValue::new(
                "aws.ecs.task.arn",
                "arn:aws:ecs:us-west-2:123456789012:task/my-task-id",
            ),
            KeyValue::new("aws.ecs.task.family", "my-task-family"),
            KeyValue::new("aws.ecs.task.revision", "5"),
            KeyValue::new("aws.ecs.launchtype", "FARGATE"),
            KeyValue::new("container.id", "abc123def456"),
            KeyValue::new("container.name", "my-container"),
            KeyValue::new("container.image.name", "my-image:latest"),
        ])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Verify ECS origin
    assert_field_eq(json, "origin", "AWS::ECS::Container");

    // Verify AWS metadata exists and contains ECS-specific data
    assert_field_exists(json, "aws");
    let aws = get_nested_value(json, "aws")
        .and_then(|v| v.as_object())
        .expect("aws should be an object");

    assert!(!aws.is_empty(), "aws object should contain ECS metadata");

    // Verify some ECS fields are present
    assert_field_exists(json, "aws.ecs");
}

#[tokio::test]
async fn test_eks_resource_metadata() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("cloud.provider", "aws"),
            KeyValue::new("cloud.platform", "aws_eks"),
            KeyValue::new("cloud.region", "eu-west-1"),
            KeyValue::new("cloud.account.id", "123456789012"),
            KeyValue::new("k8s.cluster.name", "my-eks-cluster"),
            KeyValue::new("k8s.namespace.name", "production"),
            KeyValue::new("k8s.pod.name", "my-pod-abc123"),
            KeyValue::new("k8s.deployment.name", "my-deployment"),
            KeyValue::new("container.id", "containerd://xyz789"),
        ])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Verify EKS origin
    assert_field_eq(json, "origin", "AWS::EKS::Container");

    // Verify AWS metadata exists and contains EKS-specific data
    assert_field_exists(json, "aws");
    let aws = get_nested_value(json, "aws")
        .and_then(|v| v.as_object())
        .expect("aws should be an object");

    assert!(!aws.is_empty(), "aws object should contain EKS metadata");

    // Verify some EKS fields are present
    assert_field_exists(json, "aws.eks");
}

#[tokio::test]
async fn test_lambda_resource_metadata() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("cloud.provider", "aws"),
            KeyValue::new("cloud.platform", "aws_lambda"),
            KeyValue::new("cloud.region", "ap-southeast-1"),
            KeyValue::new("cloud.account.id", "123456789012"),
            KeyValue::new("faas.name", "my-lambda-function"),
            KeyValue::new("faas.version", "$LATEST"),
            KeyValue::new("faas.instance", "2021/01/01/[$LATEST]abc123"),
            KeyValue::new("service.name", "my-lambda-function"),
        ])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Verify Lambda origin
    assert_field_eq(json, "origin", "AWS::Lambda::Function");

    // Verify AWS metadata exists
    assert_field_exists(json, "aws");
    let aws = get_nested_value(json, "aws")
        .and_then(|v| v.as_object())
        .expect("aws should be an object");

    assert!(!aws.is_empty(), "aws object should contain Lambda metadata");
}

#[tokio::test]
async fn test_elastic_beanstalk_resource_metadata() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("cloud.provider", "aws"),
            KeyValue::new("cloud.platform", "aws_elastic_beanstalk"),
            KeyValue::new("cloud.region", "us-east-1"),
            KeyValue::new("service.instance.id", "i-1234567890abcdef0"),
            KeyValue::new("service.namespace", "my-environment"),
            KeyValue::new("deployment.environment.name", "production"),
        ])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Verify Elastic Beanstalk origin
    assert_field_eq(json, "origin", "AWS::ElasticBeanstalk::Environment");
}

#[tokio::test]
async fn test_service_name_from_resource() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("service.name", "my-awesome-service"),
            KeyValue::new("service.version", "1.2.3"),
        ])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("operation", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Service data should be included
    assert_field_exists(json, "service");

    // Verify service data is an object
    let service = get_nested_value(json, "service")
        .and_then(|v| v.as_object())
        .expect("service should be an object");

    assert!(!service.is_empty(), "service object should not be empty");
}

#[tokio::test]
async fn test_telemetry_sdk_attributes() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("telemetry.sdk.name", "sdk_name"),
            KeyValue::new("telemetry.sdk.language", "sdk_language"),
            KeyValue::new("telemetry.sdk.version", "0.31.0"),
        ])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // SDK information should be in aws.xray field
    assert_field_eq(json, "aws.xray.sdk", "sdk_name for sdk_language");
    assert_field_eq(json, "aws.xray.sdk_version", "0.31.0");
}

#[tokio::test]
async fn test_cloudwatch_logs_configuration() {
    let mock_exporter = MockExporter::new();

    use opentelemetry_aws::xray_exporter::SegmentTranslator;
    let translator = SegmentTranslator::new()
        .with_log_group_name("/aws/lambda/my-function".to_string())
        .with_log_group_name("/aws/ecs/my-service".to_string());

    let mut exporter = XrayExporter::new(mock_exporter.clone()).with_translator(translator);

    let resource = Resource::builder()
        .with_attributes(vec![KeyValue::new("cloud.provider", "aws")])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // CloudWatch logs should be included
    assert_field_exists(json, "cloudwatch_logs");

    // Verify cloudwatch_logs structure
    let cw_logs = get_nested_value(json, "cloudwatch_logs")
        .and_then(|v| v.as_object())
        .expect("cloudwatch_logs should be an object");

    assert!(
        !cw_logs.is_empty(),
        "cloudwatch_logs should contain log group info"
    );
}

#[tokio::test]
async fn test_non_aws_resource() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("cloud.provider", "gcp"),
            KeyValue::new("cloud.platform", "gcp_compute_engine"),
            KeyValue::new("service.name", "my-service"),
        ])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    let json = &documents[0];

    // Should not have AWS origin
    assert_field_not_exists(json, "origin");

    // Verify basic fields are still present
    assert_field_eq(json, "name", "my-service");
    assert_field_exists(json, "id");
    assert_field_exists(json, "trace_id");
}

#[tokio::test]
async fn test_multiple_resource_updates() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    // Set initial resource
    let resource1 = Resource::builder()
        .with_attributes(vec![KeyValue::new("service.name", "service-v1")])
        .build();
    exporter.set_resource(&resource1);

    let trace_id = create_valid_trace_id();
    let span1 = create_basic_span(
        "span1",
        SpanKind::Server,
        trace_id,
        SpanId::from_bytes(0x1111111111111111u64.to_be_bytes()),
        None,
    );

    exporter.export(vec![span1]).await.unwrap();

    // Update resource
    let resource2 = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("service.name", "service-v2"),
            KeyValue::new("cloud.provider", "aws"),
            KeyValue::new("cloud.platform", "aws_lambda"),
        ])
        .build();
    exporter.set_resource(&resource2);

    let span2 = create_basic_span(
        "span2",
        SpanKind::Server,
        trace_id,
        SpanId::from_bytes(0x2222222222222222u64.to_be_bytes()),
        None,
    );

    exporter.export(vec![span2]).await.unwrap();

    let documents = mock_exporter.get_documents();
    assert_eq!(documents.len(), 2, "Should have two documents");

    // First document should not have origin
    let doc1_json = &documents[0];
    assert_field_eq(doc1_json, "name", "service-v1");
    assert_field_not_exists(doc1_json, "origin");

    // Second document should reflect updated resource
    let doc2_json = &documents[1];
    assert_field_eq(doc2_json, "name", "service-v2");
    assert_field_eq(doc2_json, "origin", "AWS::Lambda::Function");
}

#[tokio::test]
async fn test_resource_with_custom_attributes() {
    let mock_exporter = MockExporter::new();
    let mut exporter = XrayExporter::new(mock_exporter.clone());

    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("service.name", "my-service"),
            KeyValue::new("deployment.environment", "production"),
            KeyValue::new("custom.attribute", "custom-value"),
            KeyValue::new("team", "backend"),
        ])
        .build();
    exporter.set_resource(&resource);

    let trace_id = create_valid_trace_id();
    let span_id = SpanId::from_bytes(0x1111111111111111u64.to_be_bytes());
    let span = create_basic_span("test-span", SpanKind::Server, trace_id, span_id, None);

    exporter.export(vec![span]).await.unwrap();

    let documents = mock_exporter.get_documents();
    assert_eq!(documents.len(), 1, "Should export one document");

    // Verify basic structure
    let json = &documents[0];
    assert_field_eq(json, "name", "my-service");
    assert_field_exists(json, "id");
    assert_field_exists(json, "trace_id");

    // Service should be populated
    assert_field_eq(json, ["metadata", "service.name"].as_slice(), "my-service");
}
