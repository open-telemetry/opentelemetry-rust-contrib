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
