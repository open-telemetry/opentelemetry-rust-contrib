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
        false
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
