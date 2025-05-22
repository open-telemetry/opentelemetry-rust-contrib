use crate::payload_encoder::encoder::EncoderField;
use chrono::{TimeZone, Utc};
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
    // Part A region
    out[count] = EncoderField::string("env_name", "TestEnv"); // TODO - placeholder for actual env name
    count += 1;

    out[count + 1] = EncoderField::string("env_ver", "4.0");
    count += 1;

    let secs = (log.observed_time_unix_nano / 1_000_000_000) as i64;
    let nsec = (log.observed_time_unix_nano % 1_000_000_000) as u32;
    let dt = Utc
        .timestamp_opt(secs, nsec)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap())
        .to_rfc3339();
    out[count] = EncoderField::string_owned("timestamp", dt.clone());
    count += 1;
    out[count] = EncoderField::string_owned("env_time", dt);
    count += 1;

    // Part A - extenstion
    if !log.trace_id.is_empty() && count < out.len() {
        let hex = hex::encode(&log.trace_id);
        out[count] = EncoderField::string_owned("env_dt_traceId", hex);
        count += 1;
    }

    if !log.span_id.is_empty() && count < out.len() {
        let hex = hex::encode(&log.span_id);
        out[count] = EncoderField::string_owned("env_dt_spanId", hex);
        count += 1;
    }

    if count < out.len() && log.flags != 0 {
        out[count] = EncoderField::int32("env_dt_traceFlags", log.flags as i32);
        count += 1;
    }

    // Part B region
    if !log.event_name.is_empty() && count < out.len() {
        out[count] = EncoderField::string("name", &log.event_name);
        count += 1;
    }

    if count < out.len() {
        out[count] = EncoderField::int32("SeverityNumber", log.severity_number);
        count += 1;
    }

    if !log.severity_text.is_empty() && count < out.len() {
        out[count] = EncoderField::string("SeverityText", &log.severity_text);
        count += 1;
    }

    // TODO - handle all possible value types for body
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

    // Part C region
    for attr in &log.attributes {
        if count < out.len() {
            match attr.value.as_ref().and_then(|v| v.value.as_ref()) {
                Some(Value::StringValue(s)) => {
                    out[count] = EncoderField::string_with(attr.key.clone(), s.clone());
                    count += 1;
                }
                Some(Value::IntValue(i)) => {
                    out[count] = EncoderField::int32_with(attr.key.clone(), *i as i32); // TODO - handle as int64 once supported
                    count += 1;
                }
                Some(Value::DoubleValue(d)) => {
                    out[count] = EncoderField::float_with(attr.key.clone(), *d as f32); // TODO - handle as double
                    count += 1;
                }
                Some(Value::BoolValue(b)) => {
                    out[count] = EncoderField::int32_with(attr.key.clone(), if *b { 1 } else { 0 }); // TODO - handle as bool once supported
                    count += 1;
                }
                _ => {
                    // TODO - handle other types
                    // For now, we just skip them
                }
            }
        }
    }

    count
}
