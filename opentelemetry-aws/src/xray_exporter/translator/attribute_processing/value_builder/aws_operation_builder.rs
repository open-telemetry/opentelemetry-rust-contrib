use std::borrow::Cow;

use opentelemetry::Value;

use crate::xray_exporter::translator::{
    attribute_processing::{get_str, semconv, SpanAttributeProcessor},
    error::Result,
    AnyDocumentBuilder,
};

use super::ValueBuilder;

/// Builds the AWS operation name for X-Ray subsegments.
///
/// Constructs the `aws.operation` field by extracting the operation name from span attributes.
/// The builder prioritizes the `rpc.aws.operation` attribute, falling back to `rpc.method` if unavailable.
/// Only applies to subsegments.
#[derive(Debug, Default)]
pub(in crate::xray_exporter::translator) struct AwsOperationBuilder<'a> {
    aws_operation: Option<&'a str>,
    rpc_method: Option<&'a str>,
}

impl<'a> AwsOperationBuilder<'a> {
    fn aws_operation(&mut self, value: &'a Value) -> bool {
        self.aws_operation = get_str(value);
        self.aws_operation.is_some()
    }
    fn rpc_method(&mut self, value: &'a Value) -> bool {
        self.rpc_method = get_str(value);
        self.rpc_method.is_some()
    }
}

impl<'value> ValueBuilder<'value> for AwsOperationBuilder<'value> {
    fn resolve(self, segment_builder: &mut AnyDocumentBuilder<'value>) -> Result<()> {
        if let Some(operation) = self.aws_operation.or(self.rpc_method) {
            if let AnyDocumentBuilder::Subsegment(builder) = segment_builder {
                builder.aws().operation(Cow::Borrowed(operation));
            }
        }
        Ok(())
    }
}

impl<'v> SpanAttributeProcessor<'v, 2> for AwsOperationBuilder<'v> {
    const HANDLERS: [(&'static str, fn(&mut Self, &'v Value) -> bool); 2] = [
        (semconv::AWS_OPERATION, Self::aws_operation),
        (semconv::RPC_METHOD, Self::rpc_method),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xray_exporter::types::{
        Id, SegmentDocumentBuilder, SubsegmentDocumentBuilder, TraceId,
    };

    /// Finalize a subsegment builder by setting required fields, build it,
    /// and return the JSON string for assertion.
    fn build_subsegment_json(builder: AnyDocumentBuilder<'_>) -> String {
        match builder {
            AnyDocumentBuilder::Subsegment(mut b) => {
                b.name("test-subsegment").unwrap();
                b.id(Id::from(0xABCDu64));
                b.parent_id(Id::from(0x1234u64));
                b.start_time(1_000_000.0);
                b.trace_id(TraceId::new(), true).unwrap();
                b.build().unwrap().to_string()
            }
            _ => panic!("expected Subsegment variant"),
        }
    }

    /// Finalize a segment builder by setting required fields, build it,
    /// and return the JSON string for assertion.
    fn build_segment_json(builder: AnyDocumentBuilder<'_>) -> String {
        match builder {
            AnyDocumentBuilder::Segment(mut b) => {
                b.name("test-segment").unwrap();
                b.id(Id::from(0xABCDu64));
                b.start_time(1_000_000.0);
                b.trace_id(TraceId::new(), true).unwrap();
                b.build().unwrap().to_string()
            }
            _ => panic!("expected Segment variant"),
        }
    }

    /// Extract the aws.operation value from JSON output, or None if not present.
    fn extract_operation(json: &str) -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        v.get("aws")
            .and_then(|aws| aws.get("operation"))
            .and_then(|op| op.as_str())
            .map(|s| s.to_string())
    }

    #[test]
    fn test_resolve_operation_set() {
        // aws_operation set → uses it directly
        let aws_op = Value::String("DynamoDB.GetItem".into());
        let mut builder = AwsOperationBuilder::default();
        builder.aws_operation(&aws_op);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert_eq!(
            extract_operation(&json).as_deref(),
            Some("DynamoDB.GetItem"),
            "aws_operation should be used when set, got: {json}"
        );

        // Both aws_operation and rpc_method set → aws_operation takes priority
        let aws_op = Value::String("S3.PutObject".into());
        let rpc = Value::String("PutObject".into());
        let mut builder = AwsOperationBuilder::default();
        builder.aws_operation(&aws_op);
        builder.rpc_method(&rpc);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert_eq!(
            extract_operation(&json).as_deref(),
            Some("S3.PutObject"),
            "aws_operation should take priority over rpc_method, got: {json}"
        );
    }

    #[test]
    fn test_resolve_operation_fallback_and_absent() {
        // Only rpc_method set → uses it as fallback
        let rpc = Value::String("GetItem".into());
        let mut builder = AwsOperationBuilder::default();
        builder.rpc_method(&rpc);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert_eq!(
            extract_operation(&json).as_deref(),
            Some("GetItem"),
            "rpc_method should be used as fallback, got: {json}"
        );

        // Neither set → no operation field in output
        let builder = AwsOperationBuilder::default();
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert!(
            extract_operation(&json).is_none(),
            "no operation should be set when neither attribute is present, got: {json}"
        );

        // Segment builder → no operation set (subsegment-only field)
        let aws_op = Value::String("DynamoDB.Query".into());
        let mut builder = AwsOperationBuilder::default();
        builder.aws_operation(&aws_op);
        let mut doc = AnyDocumentBuilder::Segment(SegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_segment_json(doc);
        assert!(
            extract_operation(&json).is_none(),
            "segment builder should not have operation, got: {json}"
        );
    }
}
