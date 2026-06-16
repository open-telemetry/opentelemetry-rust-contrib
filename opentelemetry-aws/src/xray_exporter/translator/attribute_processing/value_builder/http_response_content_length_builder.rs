use opentelemetry::Value;

use crate::xray_exporter::translator::{
    attribute_processing::{get_integer, semconv, SpanAttributeProcessor},
    error::Result,
    AnyDocumentBuilder,
};

use super::ValueBuilder;

/// Builds the HTTP response content length for X-Ray segments and subsegments.
///
/// Constructs the `http.response.content_length` field from message payload size attributes.
/// Only includes payload size when the message type is "RECEIVED". Applies to both segments
/// and subsegments.
#[derive(Debug, Default)]
pub(in crate::xray_exporter::translator) struct HttpResponseContentLengthBuilder {
    message_type_is_received: bool,
    message_payload_size_bytes: Option<i64>,
}

impl HttpResponseContentLengthBuilder {
    fn message_type_is_received(&mut self, value: &Value) -> bool {
        if value.as_str() == "RECEIVED" {
            self.message_type_is_received = true;
        }
        false
    }
    fn message_payload_size_bytes(&mut self, value: &Value) -> bool {
        match get_integer(value) {
            Some(size) => {
                self.message_payload_size_bytes = Some(size);
                true
            }
            None => false,
        }
    }
}

impl<'value> ValueBuilder<'value> for HttpResponseContentLengthBuilder {
    fn resolve(self, segment_builder: &mut AnyDocumentBuilder<'value>) -> Result<()> {
        if self.message_type_is_received {
            let content_length = self.message_payload_size_bytes.unwrap_or_default();
            match segment_builder {
                AnyDocumentBuilder::Segment(builder) => {
                    builder.http().response.content_length(content_length);
                }
                AnyDocumentBuilder::Subsegment(builder) => {
                    builder.http().response.content_length(content_length);
                }
            }
        }

        Ok(())
    }
}

impl<'v> SpanAttributeProcessor<'v, 2> for HttpResponseContentLengthBuilder {
    const HANDLERS: [(&'static str, fn(&mut Self, &'v Value) -> bool); 2] = [
        (semconv::RPC_MESSAGE_TYPE, Self::message_type_is_received),
        (
            semconv::HTTP_RESPONSE_BODY_SIZE,
            Self::message_payload_size_bytes,
        ),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xray_exporter::types::{Id, SubsegmentDocumentBuilder, TraceId};

    /// Helper: resolve the builder into a Subsegment AnyDocumentBuilder, set required fields,
    /// build, and return the JSON string representation.
    fn build_subsegment_json(builder: HttpResponseContentLengthBuilder) -> String {
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        match doc {
            AnyDocumentBuilder::Subsegment(mut b) => {
                b.name("test").unwrap();
                b.id(Id::from(0xABCDu64));
                b.parent_id(Id::from(0x1234u64));
                b.start_time(1_000_000.0);
                b.trace_id(TraceId::new(), true).unwrap();
                b.build().unwrap().to_string()
            }
            _ => panic!("expected Subsegment variant"),
        }
    }

    #[test]
    fn test_resolve_sets_content_length_valid() {
        // RECEIVED with payload size 1024 → content_length=1024
        let msg_type = Value::String("RECEIVED".into());
        let payload_size = Value::I64(1024);
        let mut builder = HttpResponseContentLengthBuilder::default();
        builder.message_type_is_received(&msg_type);
        builder.message_payload_size_bytes(&payload_size);
        let json = build_subsegment_json(builder);
        assert!(
            json.contains("\"content_length\":1024"),
            "expected content_length=1024, got: {json}"
        );

        // RECEIVED with payload size 0 → content_length=0
        let msg_type = Value::String("RECEIVED".into());
        let payload_size = Value::I64(0);
        let mut builder = HttpResponseContentLengthBuilder::default();
        builder.message_type_is_received(&msg_type);
        builder.message_payload_size_bytes(&payload_size);
        let json = build_subsegment_json(builder);
        assert!(
            json.contains("\"content_length\":0"),
            "expected content_length=0, got: {json}"
        );

        // RECEIVED with no payload size → content_length defaults to 0
        let msg_type = Value::String("RECEIVED".into());
        let mut builder = HttpResponseContentLengthBuilder::default();
        builder.message_type_is_received(&msg_type);
        let json = build_subsegment_json(builder);
        assert!(
            json.contains("\"content_length\":0"),
            "expected content_length=0 (default), got: {json}"
        );
    }

    #[test]
    fn test_resolve_no_content_length() {
        // message_type=SENT → no content_length
        let msg_type = Value::String("SENT".into());
        let payload_size = Value::I64(512);
        let mut builder = HttpResponseContentLengthBuilder::default();
        builder.message_type_is_received(&msg_type);
        builder.message_payload_size_bytes(&payload_size);
        let json = build_subsegment_json(builder);
        assert!(
            !json.contains("content_length"),
            "expected no content_length for SENT, got: {json}"
        );

        // No message_type set at all → no content_length
        let payload_size = Value::I64(256);
        let mut builder = HttpResponseContentLengthBuilder::default();
        builder.message_payload_size_bytes(&payload_size);
        let json = build_subsegment_json(builder);
        assert!(
            !json.contains("content_length"),
            "expected no content_length without message_type, got: {json}"
        );

        // No attributes set at all → no content_length
        let builder = HttpResponseContentLengthBuilder::default();
        let json = build_subsegment_json(builder);
        assert!(
            !json.contains("content_length"),
            "expected no content_length when no attributes set, got: {json}"
        );
    }
}
