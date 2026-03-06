use opentelemetry::Value;

use crate::xray_exporter::{
    translator::{
        attribute_processing::{get_string_vec, semconv, SpanAttributeProcessor},
        error::Result,
        AnyDocumentBuilder,
    },
    types::StrList,
};

use super::ValueBuilder;

/// Builds the CloudWatch Logs log group configuration for X-Ray segments and subsegments.
///
/// Constructs the `cloudwatch_logs.log_group` field by extracting log group information from
/// span attributes. Prioritizes ARNs over names, with a fallback to a default value.
/// Applies to both segments and subsegments.
#[derive(Debug)]
pub(in crate::xray_exporter::translator) struct CloudwatchLogGroupBuilder<'a> {
    group_arns: Option<&'a dyn StrList>,
    group_names: Option<&'a dyn StrList>,
    default_value: &'a Vec<String>,
}

impl<'a> CloudwatchLogGroupBuilder<'a> {
    pub fn new(default_value: &'a Vec<String>) -> Self {
        Self {
            group_arns: None,
            group_names: None,
            default_value,
        }
    }

    fn group_arns(&mut self, value: &'a Value) -> bool {
        match get_string_vec(value) {
            Some(arns) => {
                self.group_arns = Some(arns);
                true
            }
            None => false,
        }
    }
    fn group_names(&mut self, value: &'a Value) -> bool {
        match get_string_vec(value) {
            Some(names) => {
                self.group_names = Some(names);
                true
            }
            None => false,
        }
    }
}

impl<'value> ValueBuilder<'value> for CloudwatchLogGroupBuilder<'value> {
    fn resolve(self, segment_builder: &mut AnyDocumentBuilder<'value>) -> Result<()> {
        if let Some(log_group) = self
            .group_arns
            .or(self.group_names)
            .or(Some(self.default_value))
        {
            match segment_builder {
                AnyDocumentBuilder::Segment(builder) => {
                    builder.cloudwatch_logs().log_group(log_group);
                }
                AnyDocumentBuilder::Subsegment(builder) => {
                    builder.cloudwatch_logs().log_group(log_group);
                }
            }
        }
        Ok(())
    }
}

impl<'v> SpanAttributeProcessor<'v, 2> for CloudwatchLogGroupBuilder<'v> {
    const HANDLERS: [(&'static str, fn(&mut Self, &'v Value) -> bool); 2] = [
        (semconv::AWS_LOG_GROUP_ARNS, Self::group_arns),
        (semconv::AWS_LOG_GROUP_NAMES, Self::group_names),
    ];
}
