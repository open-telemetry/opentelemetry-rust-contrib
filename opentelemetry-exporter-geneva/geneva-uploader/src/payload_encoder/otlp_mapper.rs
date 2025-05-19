use crate::payload_encoder::encoder::EncoderField;
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::logs::v1::LogRecord;

#[allow(dead_code)]
/// Converts a LogRecord to a vector of EncoderField
fn log_record_to_encoder_fields(log: &LogRecord) -> Vec<EncoderField> {
    let mut fields = Vec::new();
    // severity_number (as int32)
    fields.push(EncoderField::int32("SeverityNumber", log.severity_number));

    // severity_text (string)
    if !log.severity_text.is_empty() {
        fields.push(EncoderField::string("SeverityText", &log.severity_text));
    }

    fields.push(EncoderField::int32("Flags", log.flags as i32));

    if !log.event_name.is_empty() {
        fields.push(EncoderField::string("EventName", &log.event_name));
    }

    // body: Option<AnyValue>
    if let Some(body) = &log.body {
        // Naive - just convert string AnyValue; skip complex types for now
        if let Some(val) = &body.value {
            if let Value::StringValue(ref s) = val {
                fields.push(EncoderField::string("body", s));
            }
        }
    }

    // trace_id (as hex string, if present)
    if !log.trace_id.is_empty() {
        let hex = hex::encode(&log.trace_id);
        fields.push(EncoderField::string_owned("TraceId", hex));
    }

    // span_id (as hex string, if present)
    if !log.span_id.is_empty() {
        let hex = hex::encode(&log.span_id);
        fields.push(EncoderField::string_owned("SpanId", hex));
    }

    // Optionally: time_unix_nano as int32 (if you want; may lose precision!)
    if log.time_unix_nano < i32::MAX as u64 {
        fields.push(EncoderField::int32(
            "TimeUnixNano",
            log.time_unix_nano as i32,
        ));
    }

    fields
}
