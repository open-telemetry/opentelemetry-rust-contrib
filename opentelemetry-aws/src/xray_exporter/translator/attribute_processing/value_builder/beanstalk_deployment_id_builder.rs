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
        false
    }
    fn cloud_platform(&mut self, value: &'a Value) -> bool {
        if let Ok(Origin::Beanstalk) = value.as_str().parse() {
            self.cloud_plateform_is_beanstalk = true;
        }
        false
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
