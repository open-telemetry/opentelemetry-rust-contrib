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

        false
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
