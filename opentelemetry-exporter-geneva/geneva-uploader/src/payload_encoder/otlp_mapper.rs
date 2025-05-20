use crate::payload_encoder::encoder::EncoderField;
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::logs::v1::LogRecord;

/// Fills a mutable buffer with EncoderFields derived from LogRecord.
/// Returns the number of fields written.
/// The caller is responsible for pre-allocating the buffer with sufficient capacity.
pub(crate) fn log_record_to_encoder_fields<'a>(
    log: &'a LogRecord,
    out: &mut [EncoderField<'a>],
) -> usize {
    let mut count = 0;

    if count < out.len() {
        out[count] = EncoderField::int32("SeverityNumber", log.severity_number);
        count += 1;
    }

    if !log.severity_text.is_empty() && count < out.len() {
        out[count] = EncoderField::string("SeverityText", &log.severity_text);
        count += 1;
    }

    if count < out.len() {
        out[count] = EncoderField::int32("Flags", log.flags as i32);
        count += 1;
    }

    if !log.event_name.is_empty() && count < out.len() {
        out[count] = EncoderField::string("EventName", &log.event_name);
        count += 1;
    }

    if let Some(body) = &log.body {
        if let Some(val) = &body.value {
            if let Value::StringValue(ref s) = val {
                if count < out.len() {
                    out[count] = EncoderField::string("body", s);
                    count += 1;
                }
            }
        }
    }

    if !log.trace_id.is_empty() && count < out.len() {
        let hex = hex::encode(&log.trace_id);
        out[count] = EncoderField::string_owned("TraceId", hex);
        count += 1;
    }

    if !log.span_id.is_empty() && count < out.len() {
        let hex = hex::encode(&log.span_id);
        out[count] = EncoderField::string_owned("SpanId", hex);
        count += 1;
    }

    if log.time_unix_nano < i32::MAX as u64 && count < out.len() {
        out[count] = EncoderField::int32("TimeUnixNano", log.time_unix_nano as i32);
        count += 1;
    }

    count
}
