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
