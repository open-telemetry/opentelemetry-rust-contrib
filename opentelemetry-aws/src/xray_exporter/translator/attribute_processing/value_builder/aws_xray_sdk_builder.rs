use std::borrow::Cow;

use opentelemetry::Value;

use crate::xray_exporter::translator::{
    attribute_processing::{get_str, semconv, SpanAttributeProcessor},
    error::Result,
    AnyDocumentBuilder,
};

use super::ValueBuilder;

/// Builds the X-Ray SDK identifier for both segments and subsegments.
///
/// Constructs the `aws.xray.sdk` field by combining the SDK name and language from telemetry attributes.
/// Defaults to "opentelemetry for rust" if attributes are not present.
#[derive(Debug, Default)]
pub(in crate::xray_exporter::translator) struct AwsXraySdkBuilder<'a> {
    sdk_name: Option<&'a str>,
    sdk_lang: Option<&'a str>,
}

impl<'a> AwsXraySdkBuilder<'a> {
    fn sdk_name(&mut self, value: &'a Value) -> bool {
        self.sdk_name = get_str(value);
        self.sdk_name.is_some()
    }
    fn sdk_lang(&mut self, value: &'a Value) -> bool {
        self.sdk_lang = get_str(value);
        self.sdk_lang.is_some()
    }
}

impl<'value> ValueBuilder<'value> for AwsXraySdkBuilder<'value> {
    fn resolve(self, segment_builder: &mut AnyDocumentBuilder<'value>) -> Result<()> {
        let name = self.sdk_name.unwrap_or("opentelemetry");
        let lang = self.sdk_lang.unwrap_or("rust");
        let sdk = Cow::Owned(format!("{name} for {lang}"));
        match segment_builder {
            AnyDocumentBuilder::Segment(builder) => {
                builder.aws().xray().sdk(sdk);
            }
            AnyDocumentBuilder::Subsegment(builder) => {
                builder.aws().xray().sdk(sdk);
            }
        }
        Ok(())
    }
}

impl<'v> SpanAttributeProcessor<'v, 2> for AwsXraySdkBuilder<'v> {
    const HANDLERS: [(&'static str, fn(&mut Self, &'v Value) -> bool); 2] = [
        (semconv::TELEMETRY_SDK_NAME, Self::sdk_name),
        (semconv::TELEMETRY_SDK_LANGUAGE, Self::sdk_lang),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xray_exporter::types::{Id, SubsegmentDocumentBuilder, TraceId};

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

    /// Extract the `aws.xray.sdk` value from JSON output.
    fn extract_xray_sdk(json: &str) -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        v.get("aws")
            .and_then(|aws| aws.get("xray"))
            .and_then(|xray| xray.get("sdk"))
            .and_then(|sdk| sdk.as_str())
            .map(|s| s.to_string())
    }

    #[test]
    fn test_resolve_xray_sdk_valid() {
        // Both sdk_name and sdk_lang set → "custom-sdk for python"
        let name_val = Value::String("custom-sdk".into());
        let lang_val = Value::String("python".into());
        let mut builder = AwsXraySdkBuilder::default();
        builder.sdk_name(&name_val);
        builder.sdk_lang(&lang_val);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert_eq!(
            extract_xray_sdk(&json).as_deref(),
            Some("custom-sdk for python"),
            "both sdk_name and sdk_lang set, got: {json}"
        );

        // Neither set → defaults to "opentelemetry for rust"
        let builder = AwsXraySdkBuilder::default();
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert_eq!(
            extract_xray_sdk(&json).as_deref(),
            Some("opentelemetry for rust"),
            "neither set should default to 'opentelemetry for rust', got: {json}"
        );
    }

    #[test]
    fn test_resolve_xray_sdk_partial() {
        // Only sdk_name set → "my-otel for rust" (default lang)
        let name_val = Value::String("my-otel".into());
        let mut builder = AwsXraySdkBuilder::default();
        builder.sdk_name(&name_val);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert_eq!(
            extract_xray_sdk(&json).as_deref(),
            Some("my-otel for rust"),
            "only sdk_name set should default lang to 'rust', got: {json}"
        );

        // Only sdk_lang set → "opentelemetry for java" (default name)
        let lang_val = Value::String("java".into());
        let mut builder = AwsXraySdkBuilder::default();
        builder.sdk_lang(&lang_val);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert_eq!(
            extract_xray_sdk(&json).as_deref(),
            Some("opentelemetry for java"),
            "only sdk_lang set should default name to 'opentelemetry', got: {json}"
        );
    }
}
