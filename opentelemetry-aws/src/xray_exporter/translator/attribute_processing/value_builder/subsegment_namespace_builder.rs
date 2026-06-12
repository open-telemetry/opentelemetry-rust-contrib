use opentelemetry::Value;

use crate::xray_exporter::{
    translator::{
        attribute_processing::{semconv, SpanAttributeProcessor},
        error::Result,
        AnyDocumentBuilder,
    },
    types::Namespace,
};

use super::ValueBuilder;

/// Builds the namespace field for X-Ray subsegments.
///
/// Constructs the `namespace` field to categorize subsegments as "aws" (for AWS service calls) or
/// "remote" (for client spans calling external services). The namespace helps X-Ray identify the type
/// of downstream call. Only applies to subsegments.
#[derive(Debug, Default)]
pub(in crate::xray_exporter::translator) struct SubsegmentNamespaceBuilder {
    rpc_system_is_aws_api: bool,
    aws_service_is_some: bool,
    span_kind_is_client: bool,
}

impl SubsegmentNamespaceBuilder {
    pub fn new(span_kind_is_client: bool) -> Self {
        Self {
            span_kind_is_client,
            ..Default::default()
        }
    }

    fn rpc_system_is_aws_api(&mut self, value: &Value) -> bool {
        if value.as_str() == "aws-api" {
            self.rpc_system_is_aws_api = true;
        }
        self.rpc_system_is_aws_api
    }
    fn aws_service_is_some(&mut self, value: &Value) -> bool {
        if !value.as_str().is_empty() {
            self.aws_service_is_some = true;
        }
        false
    }
}

impl<'value> ValueBuilder<'value> for SubsegmentNamespaceBuilder {
    fn resolve(self, segment_builder: &mut AnyDocumentBuilder<'value>) -> Result<()> {
        let namespace = if self.rpc_system_is_aws_api || self.aws_service_is_some {
            Some(Namespace::Aws)
        } else if self.span_kind_is_client {
            Some(Namespace::Remote)
        } else {
            None
        };
        if let Some(namespace) = namespace {
            if let AnyDocumentBuilder::Subsegment(builder) = segment_builder {
                builder.namespace(namespace);
            }
        }
        Ok(())
    }
}
impl<'v> SpanAttributeProcessor<'v, 2> for SubsegmentNamespaceBuilder {
    const HANDLERS: [(&'static str, fn(&mut Self, &'v Value) -> bool); 2] = [
        (semconv::RPC_SYSTEM, Self::rpc_system_is_aws_api),
        (semconv::AWS_SERVICE, Self::aws_service_is_some),
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

    /// Extract the namespace value from JSON output, or None if not present.
    fn extract_namespace(json: &str) -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        v.get("namespace")
            .and_then(|n| n.as_str())
            .map(|s| s.to_string())
    }

    #[test]
    fn test_resolve_namespace_aws() {
        // rpc_system = "aws-api" → Namespace::Aws ("aws")
        let rpc_system = Value::String("aws-api".into());
        let mut builder = SubsegmentNamespaceBuilder::new(false);
        builder.rpc_system_is_aws_api(&rpc_system);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert_eq!(
            extract_namespace(&json).as_deref(),
            Some("aws"),
            "rpc_system=aws-api should produce namespace 'aws', got: {json}"
        );

        // aws_service is set (non-empty) → Namespace::Aws ("aws")
        let aws_service = Value::String("DynamoDB".into());
        let mut builder = SubsegmentNamespaceBuilder::new(false);
        builder.aws_service_is_some(&aws_service);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert_eq!(
            extract_namespace(&json).as_deref(),
            Some("aws"),
            "aws_service set should produce namespace 'aws', got: {json}"
        );

        // Both rpc_system=aws-api AND aws_service set → still Namespace::Aws
        let rpc_system = Value::String("aws-api".into());
        let aws_service = Value::String("S3".into());
        let mut builder = SubsegmentNamespaceBuilder::new(true);
        builder.rpc_system_is_aws_api(&rpc_system);
        builder.aws_service_is_some(&aws_service);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert_eq!(
            extract_namespace(&json).as_deref(),
            Some("aws"),
            "both aws indicators should produce namespace 'aws', got: {json}"
        );
    }

    #[test]
    fn test_resolve_namespace_remote_and_none() {
        // span_kind_is_client=true, no AWS indicators → Namespace::Remote ("remote")
        let builder = SubsegmentNamespaceBuilder::new(true);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert_eq!(
            extract_namespace(&json).as_deref(),
            Some("remote"),
            "client span without aws indicators should produce namespace 'remote', got: {json}"
        );

        // span_kind_is_client=false, no AWS indicators → no namespace
        let builder = SubsegmentNamespaceBuilder::new(false);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert!(
            extract_namespace(&json).is_none(),
            "non-client span without aws indicators should have no namespace, got: {json}"
        );

        // rpc_system is set but NOT "aws-api" + not client → no namespace
        let rpc_system = Value::String("grpc".into());
        let mut builder = SubsegmentNamespaceBuilder::new(false);
        builder.rpc_system_is_aws_api(&rpc_system);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert!(
            extract_namespace(&json).is_none(),
            "non-aws rpc_system without client should have no namespace, got: {json}"
        );

        // Segment builder → no namespace set (subsegment-only field)
        let rpc_system = Value::String("aws-api".into());
        let mut builder = SubsegmentNamespaceBuilder::new(true);
        builder.rpc_system_is_aws_api(&rpc_system);
        let mut doc = AnyDocumentBuilder::Segment(SegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_segment_json(doc);
        assert!(
            extract_namespace(&json).is_none(),
            "segment builder should not have namespace, got: {json}"
        );
    }
}
