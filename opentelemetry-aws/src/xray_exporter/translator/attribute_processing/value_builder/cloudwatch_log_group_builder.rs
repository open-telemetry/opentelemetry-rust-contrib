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
        if let Some(log_groups) = self
            .group_arns
            .or(self.group_names)
            .or(Some(self.default_value))
        {
            match segment_builder {
                AnyDocumentBuilder::Segment(builder) => {
                    builder.cloudwatch_logs().log_group(log_groups);
                }
                AnyDocumentBuilder::Subsegment(builder) => {
                    builder.cloudwatch_logs().log_group(log_groups);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xray_exporter::types::{
        Id, SegmentDocumentBuilder, SubsegmentDocumentBuilder, TraceId,
    };
    use opentelemetry::Array;

    /// Resolve the builder into a Segment, set required fields, build, and return JSON.
    fn build_segment_json(builder: CloudwatchLogGroupBuilder<'_>) -> String {
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

    /// Resolve the builder into a Subsegment, set required fields, build, and return JSON.
    fn build_subsegment_json(builder: CloudwatchLogGroupBuilder<'_>) -> String {
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        match doc {
            AnyDocumentBuilder::Subsegment(mut b) => {
                b.name("test-sub").unwrap();
                b.id(Id::from(0xABCDu64));
                b.parent_id(Id::from(0x1234u64));
                b.start_time(1_000_000.0);
                b.trace_id(TraceId::new(), true).unwrap();
                b.build().unwrap().to_string()
            }
            _ => panic!("expected Subsegment variant"),
        }
    }

    /// Extract the cloudwatch_logs.log_group array from JSON, or None if absent.
    fn extract_log_groups(json: &str) -> Option<Vec<String>> {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        v.get("cloudwatch_logs")
            .and_then(|cw| cw.get("log_group"))
            .and_then(|lg| lg.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect()
            })
    }

    #[test]
    fn test_resolve_log_group_valid() {
        // group_arns set → uses arns as log_group
        let arns_value = Value::Array(Array::String(vec![
            "arn:aws:logs:us-east-1:123456789012:log-group:my-group".into(),
        ]));
        let default = vec![];
        let mut builder = CloudwatchLogGroupBuilder::new(&default);
        builder.group_arns(&arns_value);
        let json = build_segment_json(builder);
        let groups =
            extract_log_groups(&json).expect("cloudwatch_logs.log_group should be present");
        assert_eq!(
            groups,
            vec!["arn:aws:logs:us-east-1:123456789012:log-group:my-group"],
            "group_arns should be used, got: {json}"
        );

        // Only group_names set → uses names as log_group
        let names_value = Value::Array(Array::String(vec![
            "my-log-group".into(),
            "another-group".into(),
        ]));
        let default = vec![];
        let mut builder = CloudwatchLogGroupBuilder::new(&default);
        builder.group_names(&names_value);
        let json = build_segment_json(builder);
        let groups =
            extract_log_groups(&json).expect("cloudwatch_logs.log_group should be present");
        assert_eq!(
            groups,
            vec!["my-log-group", "another-group"],
            "group_names should be used as fallback, got: {json}"
        );

        // Neither arns nor names set → uses default_value
        let default = vec!["default-log-group".to_string()];
        let builder = CloudwatchLogGroupBuilder::new(&default);
        let json = build_segment_json(builder);
        let groups =
            extract_log_groups(&json).expect("cloudwatch_logs.log_group should be present");
        assert_eq!(
            groups,
            vec!["default-log-group"],
            "default_value should be used when no attributes set, got: {json}"
        );

        // Both arns and names set → arns take priority
        let arns_value = Value::Array(Array::String(vec![
            "arn:aws:logs:us-west-2:111111111111:log-group:priority-group".into(),
        ]));
        let names_value = Value::Array(Array::String(vec!["should-not-appear".into()]));
        let default = vec![];
        let mut builder = CloudwatchLogGroupBuilder::new(&default);
        builder.group_arns(&arns_value);
        builder.group_names(&names_value);
        let json = build_segment_json(builder);
        let groups =
            extract_log_groups(&json).expect("cloudwatch_logs.log_group should be present");
        assert_eq!(
            groups,
            vec!["arn:aws:logs:us-west-2:111111111111:log-group:priority-group"],
            "group_arns should take priority over group_names, got: {json}"
        );

        // Subsegment variant also works
        let arns_value = Value::Array(Array::String(vec!["arn:subsegment-group".into()]));
        let default = vec![];
        let mut builder = CloudwatchLogGroupBuilder::new(&default);
        builder.group_arns(&arns_value);
        let json = build_subsegment_json(builder);
        let groups = extract_log_groups(&json)
            .expect("cloudwatch_logs.log_group should be present in subsegment");
        assert_eq!(
            groups,
            vec!["arn:subsegment-group"],
            "subsegment should also get log_group, got: {json}"
        );
    }

    #[test]
    fn test_resolve_log_group_absent() {
        // Empty default_value and no attributes → log_group is empty array, skipped by MaybeSkip
        let default: Vec<String> = vec![];
        let builder = CloudwatchLogGroupBuilder::new(&default);
        let json = build_segment_json(builder);
        assert!(
            extract_log_groups(&json).is_none(),
            "cloudwatch_logs should be absent when default is empty and no attributes set, got: {json}"
        );

        // Non-string-array value is rejected by group_arns handler
        let bad_value = Value::I64(42);
        let default: Vec<String> = vec![];
        let mut builder = CloudwatchLogGroupBuilder::new(&default);
        let accepted = builder.group_arns(&bad_value);
        assert!(
            !accepted,
            "group_arns should reject non-string-array values"
        );
        let json = build_segment_json(builder);
        assert!(
            extract_log_groups(&json).is_none(),
            "cloudwatch_logs should be absent when handler rejects value, got: {json}"
        );

        // Non-string-array value is rejected by group_names handler
        let bad_value = Value::Bool(true);
        let default: Vec<String> = vec![];
        let mut builder = CloudwatchLogGroupBuilder::new(&default);
        let accepted = builder.group_names(&bad_value);
        assert!(
            !accepted,
            "group_names should reject non-string-array values"
        );
        let json = build_segment_json(builder);
        assert!(
            extract_log_groups(&json).is_none(),
            "cloudwatch_logs should be absent when handler rejects value, got: {json}"
        );
    }
}
