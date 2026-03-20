use opentelemetry::Value;

use crate::xray_exporter::{
    translator::{
        attribute_processing::{get_str, semconv, SpanAttributeProcessor},
        error::Result,
        AnyDocumentBuilder,
    },
    types::Origin,
};

use super::ValueBuilder;

/// Builds the origin field for X-Ray segments running on AWS platforms.
///
/// Constructs the `origin` field by parsing the cloud platform attribute when the cloud provider is AWS.
/// The origin identifies the AWS compute platform (e.g., EC2, ECS, Lambda) where the traced application runs.
/// Only applies to segments.
#[derive(Debug, Default)]
pub(in crate::xray_exporter::translator) struct SegmentOriginBuilder<'a> {
    cloud_platform: Option<&'a str>,
    cloud_provider_is_aws: bool,
    ecs_launch_type: Option<&'a str>,
}
impl<'a> SegmentOriginBuilder<'a> {
    fn cloud_provider_is_aws(&mut self, value: &'a Value) -> bool {
        if value.as_str() == "aws" {
            self.cloud_provider_is_aws = true;
        }
        self.cloud_provider_is_aws
    }
    fn cloud_platform(&mut self, value: &'a Value) -> bool {
        self.cloud_platform = get_str(value);
        self.cloud_platform.is_some()
    }
    fn ecs_launch_type(&mut self, value: &'a Value) -> bool {
        self.ecs_launch_type = get_str(value);
        self.ecs_launch_type.is_some()
    }
}

impl<'value> ValueBuilder<'value> for SegmentOriginBuilder<'value> {
    fn resolve(self, segment_builder: &mut AnyDocumentBuilder<'value>) -> Result<()> {
        if self.cloud_provider_is_aws {
            if let Some(cloud_platform) = self.cloud_platform {
                if let Ok(origin) = cloud_platform.parse() {
                    let origin = match (self.ecs_launch_type, origin) {
                        (Some("ec2"), Origin::Ecs) => Origin::EcsEc2,
                        (Some("fargate"), Origin::Ecs) => Origin::EcsFargate,
                        _ => origin,
                    };
                    if let AnyDocumentBuilder::Segment(builder) = segment_builder {
                        builder.origin(origin);
                    }
                }
            }
        }
        Ok(())
    }
}

impl<'v> SpanAttributeProcessor<'v, 3> for SegmentOriginBuilder<'v> {
    const HANDLERS: [(&'static str, fn(&mut Self, &'v Value) -> bool); 3] = [
        (semconv::CLOUD_PROVIDER, Self::cloud_provider_is_aws),
        (semconv::CLOUD_PLATFORM, Self::cloud_platform),
        (semconv::AWS_ECS_LAUNCHTYPE, Self::ecs_launch_type),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xray_exporter::types::{
        Id, SegmentDocumentBuilder, SubsegmentDocumentBuilder, TraceId,
    };
    use opentelemetry::Value;

    /// Helper: resolve the builder into an AnyDocumentBuilder, set required fields, build, and
    /// return the JSON string representation.
    fn build_segment_json(builder: SegmentOriginBuilder<'_>) -> String {
        let mut doc = AnyDocumentBuilder::Segment(SegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        match doc {
            AnyDocumentBuilder::Segment(mut b) => {
                b.name("test").unwrap();
                b.id(Id::from(0xABCDu64));
                b.start_time(1_000_000.0);
                b.trace_id(TraceId::new(), true).unwrap();
                b.build().unwrap().to_string()
            }
            _ => panic!("expected Segment variant"),
        }
    }

    #[test]
    fn test_resolve_valid_origins() {
        // ECS with EC2 launch type → "AWS::ECS::EC2"
        let provider = Value::String("aws".into());
        let platform = Value::String("aws_ecs".into());
        let launch = Value::String("ec2".into());
        let mut builder = SegmentOriginBuilder::default();
        builder.cloud_provider_is_aws(&provider);
        builder.cloud_platform(&platform);
        builder.ecs_launch_type(&launch);
        let json = build_segment_json(builder);
        assert!(
            json.contains("\"origin\":\"AWS::ECS::EC2\""),
            "expected ECS EC2 origin, got: {json}"
        );

        // ECS with Fargate launch type → "AWS::ECS::Fargate"
        let provider = Value::String("aws".into());
        let platform = Value::String("aws_ecs".into());
        let launch = Value::String("fargate".into());
        let mut builder = SegmentOriginBuilder::default();
        builder.cloud_provider_is_aws(&provider);
        builder.cloud_platform(&platform);
        builder.ecs_launch_type(&launch);
        let json = build_segment_json(builder);
        assert!(
            json.contains("\"origin\":\"AWS::ECS::Fargate\""),
            "expected ECS Fargate origin, got: {json}"
        );

        // EC2 platform (no ECS launch type) → "AWS::EC2::Instance"
        let provider = Value::String("aws".into());
        let platform = Value::String("aws_ec2".into());
        let mut builder = SegmentOriginBuilder::default();
        builder.cloud_provider_is_aws(&provider);
        builder.cloud_platform(&platform);
        let json = build_segment_json(builder);
        assert!(
            json.contains("\"origin\":\"AWS::EC2::Instance\""),
            "expected EC2 Instance origin, got: {json}"
        );

        // ECS without launch type → "AWS::ECS::Container" (base ECS origin)
        let provider = Value::String("aws".into());
        let platform = Value::String("aws_ecs".into());
        let mut builder = SegmentOriginBuilder::default();
        builder.cloud_provider_is_aws(&provider);
        builder.cloud_platform(&platform);
        let json = build_segment_json(builder);
        assert!(
            json.contains("\"origin\":\"AWS::ECS::Container\""),
            "expected ECS Container origin, got: {json}"
        );
    }

    #[test]
    fn test_resolve_no_origin_set() {
        // Non-AWS cloud provider → no origin field
        let provider = Value::String("gcp".into());
        let platform = Value::String("aws_ec2".into());
        let mut builder = SegmentOriginBuilder::default();
        builder.cloud_provider_is_aws(&provider);
        builder.cloud_platform(&platform);
        let json = build_segment_json(builder);
        assert!(
            !json.contains("\"origin\""),
            "expected no origin for non-AWS provider, got: {json}"
        );

        // AWS provider but unrecognized platform → no origin field
        let provider = Value::String("aws".into());
        let platform = Value::String("unknown_platform".into());
        let mut builder = SegmentOriginBuilder::default();
        builder.cloud_provider_is_aws(&provider);
        builder.cloud_platform(&platform);
        let json = build_segment_json(builder);
        assert!(
            !json.contains("\"origin\""),
            "expected no origin for unknown platform, got: {json}"
        );

        // No attributes set at all → no origin field
        let builder = SegmentOriginBuilder::default();
        let json = build_segment_json(builder);
        assert!(
            !json.contains("\"origin\""),
            "expected no origin when no attributes set, got: {json}"
        );

        // Subsegment builder → origin not set (segment-only field)
        let provider = Value::String("aws".into());
        let platform = Value::String("aws_ec2".into());
        let mut builder = SegmentOriginBuilder::default();
        builder.cloud_provider_is_aws(&provider);
        builder.cloud_platform(&platform);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        match doc {
            AnyDocumentBuilder::Subsegment(mut b) => {
                b.name("test").unwrap();
                b.id(Id::from(0xABCDu64));
                b.parent_id(Id::from(0x1234u64));
                b.start_time(1_000_000.0);
                b.trace_id(TraceId::new(), true).unwrap();
                let json = b.build().unwrap().to_string();
                assert!(
                    !json.contains("\"origin\""),
                    "expected no origin on subsegment, got: {json}"
                );
            }
            _ => panic!("expected Subsegment variant"),
        }
    }
}
