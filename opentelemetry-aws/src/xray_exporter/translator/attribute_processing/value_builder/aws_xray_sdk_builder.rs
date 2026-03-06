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
