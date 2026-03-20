use std::borrow::Cow;

use opentelemetry::Value;

use crate::xray_exporter::{
    translator::{
        attribute_processing::{get_cow, semconv, SpanAttributeProcessor},
        error::Result,
        AnyDocumentBuilder,
    },
    types::Origin,
};

use super::ValueBuilder;

/// Builds the Elastic Beanstalk deployment ID for X-Ray segments.
///
/// Constructs the `aws.elastic_beanstalk.deployment_id` field by parsing the service instance ID
/// when running on AWS Elastic Beanstalk. Only applies to segments when both the cloud provider is
/// AWS and the cloud platform is Elastic Beanstalk.
#[derive(Debug, Default)]
pub(in crate::xray_exporter::translator) struct BeanstalkDeploymentIdBuilder<'a> {
    cloud_provider_is_aws: bool,
    cloud_plateform_is_beanstalk: bool,
    service_instance_id: Option<Cow<'a, str>>,
}

impl<'a> BeanstalkDeploymentIdBuilder<'a> {
    fn cloud_provider(&mut self, value: &'a Value) -> bool {
        if value.as_str() == "aws" {
            self.cloud_provider_is_aws = true;
        }
        self.cloud_provider_is_aws
    }
    fn cloud_platform(&mut self, value: &'a Value) -> bool {
        if let Ok(Origin::Beanstalk) = value.as_str().parse() {
            self.cloud_plateform_is_beanstalk = true;
        }
        self.cloud_plateform_is_beanstalk
    }
    fn service_instance_id(&mut self, value: &'a Value) -> bool {
        self.service_instance_id = Some(get_cow(value));
        true
    }
}

impl<'value> ValueBuilder<'value> for BeanstalkDeploymentIdBuilder<'value> {
    fn resolve(self, segment_builder: &mut AnyDocumentBuilder) -> Result<()> {
        if self.cloud_plateform_is_beanstalk {
            if let Some(service_instance_id) = self.service_instance_id {
                if let Ok(deployment_id) = service_instance_id.as_ref().parse() {
                    if let AnyDocumentBuilder::Segment(builder) = segment_builder {
                        builder
                            .aws()
                            .elastic_beanstalk()
                            .deployment_id(deployment_id);
                    }
                }
            }
        }
        Ok(())
    }
}

impl<'v> SpanAttributeProcessor<'v, 3> for BeanstalkDeploymentIdBuilder<'v> {
    const HANDLERS: [(&'static str, fn(&mut Self, &'v Value) -> bool); 3] = [
        (semconv::CLOUD_PROVIDER, Self::cloud_provider),
        (semconv::CLOUD_PLATFORM, Self::cloud_platform),
        (semconv::SERVICE_INSTANCE_ID, Self::service_instance_id),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xray_exporter::types::{Id, SegmentDocumentBuilder, TraceId};

    /// Helper: resolve the builder into a Segment AnyDocumentBuilder, set required fields,
    /// build, and return the JSON string representation.
    fn build_segment_json(builder: BeanstalkDeploymentIdBuilder<'_>) -> String {
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
    fn test_resolve_sets_deployment_id_valid() {
        // aws + aws_elastic_beanstalk + parseable i64 → deployment_id present
        let provider = Value::String("aws".into());
        let platform = Value::String("aws_elastic_beanstalk".into());
        let instance_id = Value::String("42".into());
        let mut builder = BeanstalkDeploymentIdBuilder::default();
        builder.cloud_provider(&provider);
        builder.cloud_platform(&platform);
        builder.service_instance_id(&instance_id);
        let json = build_segment_json(builder);
        assert!(
            json.contains("\"deployment_id\":42"),
            "expected deployment_id=42, got: {json}"
        );

        // Negative deployment ID is also a valid i64
        let provider = Value::String("aws".into());
        let platform = Value::String("aws_elastic_beanstalk".into());
        let instance_id = Value::String("-7".into());
        let mut builder = BeanstalkDeploymentIdBuilder::default();
        builder.cloud_provider(&provider);
        builder.cloud_platform(&platform);
        builder.service_instance_id(&instance_id);
        let json = build_segment_json(builder);
        assert!(
            json.contains("\"deployment_id\":-7"),
            "expected deployment_id=-7, got: {json}"
        );

        // Zero is a valid deployment ID
        let provider = Value::String("aws".into());
        let platform = Value::String("aws_elastic_beanstalk".into());
        let instance_id = Value::String("0".into());
        let mut builder = BeanstalkDeploymentIdBuilder::default();
        builder.cloud_provider(&provider);
        builder.cloud_platform(&platform);
        builder.service_instance_id(&instance_id);
        let json = build_segment_json(builder);
        assert!(
            json.contains("\"deployment_id\":0"),
            "expected deployment_id=0, got: {json}"
        );
    }

    #[test]
    fn test_resolve_no_deployment_id() {
        // Non-beanstalk platform (aws_ecs) → no deployment_id
        let provider = Value::String("aws".into());
        let platform = Value::String("aws_ecs".into());
        let instance_id = Value::String("42".into());
        let mut builder = BeanstalkDeploymentIdBuilder::default();
        builder.cloud_provider(&provider);
        builder.cloud_platform(&platform);
        builder.service_instance_id(&instance_id);
        let json = build_segment_json(builder);
        assert!(
            !json.contains("deployment_id"),
            "expected no deployment_id for non-beanstalk platform, got: {json}"
        );

        // Unparseable service_instance_id → no deployment_id
        let provider = Value::String("aws".into());
        let platform = Value::String("aws_elastic_beanstalk".into());
        let instance_id = Value::String("abc".into());
        let mut builder = BeanstalkDeploymentIdBuilder::default();
        builder.cloud_provider(&provider);
        builder.cloud_platform(&platform);
        builder.service_instance_id(&instance_id);
        let json = build_segment_json(builder);
        assert!(
            !json.contains("deployment_id"),
            "expected no deployment_id for non-numeric instance id, got: {json}"
        );

        // No attributes set at all → no deployment_id
        let builder = BeanstalkDeploymentIdBuilder::default();
        let json = build_segment_json(builder);
        assert!(
            !json.contains("deployment_id"),
            "expected no deployment_id when no attributes set, got: {json}"
        );

        // Beanstalk platform but no service_instance_id → no deployment_id
        let platform = Value::String("aws_elastic_beanstalk".into());
        let mut builder = BeanstalkDeploymentIdBuilder::default();
        builder.cloud_platform(&platform);
        let json = build_segment_json(builder);
        assert!(
            !json.contains("deployment_id"),
            "expected no deployment_id without service_instance_id, got: {json}"
        );
    }
}
