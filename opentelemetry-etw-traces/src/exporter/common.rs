use opentelemetry::trace::SpanKind;
use std::borrow::Cow;
use std::time::SystemTime;
use tracelogging_dynamic as tld;

pub(crate) fn format_trace_id(id: opentelemetry::TraceId) -> [u8; 32] {
    let mut buffer = [0; 32];
    encode_hex(&id.to_bytes(), &mut buffer);
    buffer
}

pub(crate) fn format_span_id(id: opentelemetry::SpanId) -> [u8; 16] {
    let mut buffer = [0; 16];
    encode_hex(&id.to_bytes(), &mut buffer);
    buffer
}

fn encode_hex(bytes: &[u8], buffer: &mut [u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    debug_assert_eq!(buffer.len(), bytes.len() * 2);
    for (byte, pair) in bytes.iter().zip(buffer.chunks_exact_mut(2)) {
        pair[0] = HEX[(byte >> 4) as usize];
        pair[1] = HEX[(byte & 0x0f) as usize];
    }
}

pub(crate) struct TimestampBuffer {
    // Chrono supports signed, six-digit years in addition to four-digit years.
    bytes: [u8; 36],
    len: usize,
}

impl TimestampBuffer {
    fn write_digits(&mut self, mut value: u32, width: usize) {
        let end = self.len + width;
        for digit in self.bytes[self.len..end].iter_mut().rev() {
            *digit = b'0' + (value % 10) as u8;
            value /= 10;
        }
        self.len = end;
    }
}

impl AsRef<[u8]> for TimestampBuffer {
    fn as_ref(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

pub(crate) fn format_timestamp(time: SystemTime) -> TimestampBuffer {
    use chrono::{Datelike, Timelike};

    let datetime: chrono::DateTime<chrono::Utc> = time.into();
    let mut buffer = TimestampBuffer {
        bytes: [0; 36],
        len: 0,
    };
    let year = datetime.year();
    if !(0..=9999).contains(&year) {
        buffer.bytes[0] = if year < 0 { b'-' } else { b'+' };
        buffer.len = 1;
    }
    let year = year.unsigned_abs();
    let year_width = if year < 10_000 {
        4
    } else if year < 100_000 {
        5
    } else {
        6
    };
    buffer.write_digits(year, year_width);

    let offset = buffer.len;
    buffer.bytes[offset..offset + 15].copy_from_slice(b"-00-00T00:00:00");
    for (position, value) in [
        (1, datetime.month()),
        (4, datetime.day()),
        (7, datetime.hour()),
        (10, datetime.minute()),
        (13, datetime.second()),
    ] {
        buffer.bytes[offset + position] = b'0' + (value / 10) as u8;
        buffer.bytes[offset + position + 1] = b'0' + (value % 10) as u8;
    }
    buffer.len += 15;

    let nanos = datetime.nanosecond();
    if nanos != 0 {
        buffer.bytes[buffer.len] = b'.';
        buffer.len += 1;
        // Match SecondsFormat::AutoSi: use exactly 3, 6, or 9 fractional digits.
        if nanos % 1_000_000 == 0 {
            buffer.write_digits(nanos / 1_000_000, 3);
        } else if nanos % 1_000 == 0 {
            buffer.write_digits(nanos / 1_000, 6);
        } else {
            buffer.write_digits(nanos, 9);
        }
    }
    buffer.bytes[buffer.len] = b'Z';
    buffer.len += 1;
    buffer
}

/// Well-known OpenTelemetry semantic-convention attributes that map to
/// Common Schema Part B field names. Attributes whose key matches one of
/// these entries are emitted as Part B fields (using the mapped name)
/// instead of Part C.
///
/// Stored as a static slice (rather than a `HashMap`) to guarantee a
/// stable, deterministic ordering of fields in the emitted ETW event.
pub(crate) const WELL_KNOWN_PART_B_ATTRIBUTES: &[(&str, &str)] = &[
    // Database
    ("db.system.name", "dbSystem"),
    ("db.namespace", "dbName"),
    ("db.query.text", "dbStatement"),
    // HTTP
    ("http.request.method", "httpMethod"),
    ("url.full", "httpUrl"),
    ("http.response.status_code", "httpStatusCode"),
    // Messaging
    ("messaging.system", "messagingSystem"),
    ("messaging.destination.name", "messagingDestination"),
    ("messaging.url", "messagingUrl"),
    // RPC attributes
    ("rpc.system.name", "rpcSystem"),
    ("rpc.response.status_code", "rpcGrpcStatusCode"),
];

/// Returns the mapped Part B field name if `key` is a well-known attribute,
/// otherwise `None`.
#[inline]
pub(crate) fn well_known_part_b_field(key: &str) -> Option<&'static str> {
    WELL_KNOWN_PART_B_ATTRIBUTES
        .iter()
        .find_map(|(otel_key, partb_name)| (*otel_key == key).then_some(*partb_name))
}

/// Converts an OpenTelemetry `SpanKind` to a u8 value matching the OTel spec.
pub(crate) fn span_kind_to_u8(kind: &SpanKind) -> u8 {
    match kind {
        SpanKind::Internal => 0,
        SpanKind::Server => 1,
        SpanKind::Client => 2,
        SpanKind::Producer => 3,
        SpanKind::Consumer => 4,
    }
}

/// Adds an OpenTelemetry attribute value to a TLD EventBuilder as a typed field.
pub(crate) fn add_attribute_to_event(
    event: &mut tld::EventBuilder,
    field_name: &str,
    value: &opentelemetry::Value,
) {
    match value {
        opentelemetry::Value::Bool(b) => {
            event.add_bool32(field_name, *b as i32, tld::OutType::Default, 0);
        }
        opentelemetry::Value::I64(i) => {
            event.add_i64(field_name, *i, tld::OutType::Default, 0);
        }
        opentelemetry::Value::F64(f) => {
            event.add_f64(field_name, *f, tld::OutType::Default, 0);
        }
        opentelemetry::Value::String(s) => {
            event.add_str8(field_name, s.as_str(), tld::OutType::Default, 0);
        }
        #[cfg(feature = "serde_json")]
        opentelemetry::Value::Array(arr) => {
            let json = array_to_json(arr);
            event.add_str8(field_name, &json, tld::OutType::Json, 0);
        }
        _ => {
            event.add_str8(field_name, "", tld::OutType::Default, 0);
        }
    }
}

#[cfg(feature = "serde_json")]
fn array_to_value(arr: &opentelemetry::Array) -> serde_json::Value {
    use opentelemetry::Array;
    match arr {
        Array::Bool(v) => {
            serde_json::Value::Array(v.iter().map(|b| serde_json::Value::Bool(*b)).collect())
        }
        Array::I64(v) => serde_json::Value::Array(
            v.iter()
                .map(|i| serde_json::Value::Number((*i).into()))
                .collect(),
        ),
        Array::F64(v) => serde_json::Value::Array(
            v.iter()
                .map(|f| {
                    serde_json::Number::from_f64(*f)
                        .map(serde_json::Value::Number)
                        .unwrap_or(serde_json::Value::Null)
                })
                .collect(),
        ),
        Array::String(v) => serde_json::Value::Array(
            v.iter()
                .map(|s| serde_json::Value::String(s.to_string()))
                .collect(),
        ),
        _ => serde_json::Value::Array(vec![]),
    }
}

#[cfg(feature = "serde_json")]
fn array_to_json(arr: &opentelemetry::Array) -> String {
    serde_json::to_string(&array_to_value(arr)).unwrap_or_default()
}

/// Serializes span links to a JSON string (array of {toTraceId, toSpanId}).
/// Without the `serde_json` feature, always returns `"[]"`.
pub(crate) fn links_to_json(links: &[opentelemetry::trace::Link]) -> Cow<'static, str> {
    #[cfg(feature = "serde_json")]
    {
        if links.is_empty() {
            return Cow::Borrowed("[]");
        }
        let arr: Vec<serde_json::Value> = links
            .iter()
            .map(|link| {
                serde_json::json!({
                    "toTraceId": link.span_context.trace_id().to_string(),
                    "toSpanId": link.span_context.span_id().to_string(),
                })
            })
            .collect();
        Cow::Owned(serde_json::to_string(&arr).unwrap_or_default())
    }
    #[cfg(not(feature = "serde_json"))]
    {
        let _ = links;
        Cow::Borrowed("[]")
    }
}

#[cfg(test)]
pub(crate) mod test_utils {
    use super::super::options::Options;
    use super::super::ETWExporter;
    use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
    use opentelemetry::KeyValue;
    use opentelemetry_sdk::trace::SpanData;

    pub(crate) fn new_etw_exporter() -> ETWExporter {
        ETWExporter::new(test_options())
    }

    pub(crate) fn test_options() -> Options {
        Options::new("TestProvider")
    }

    pub(crate) fn create_test_span_data(attributes: Option<Vec<KeyValue>>) -> SpanData {
        use opentelemetry::trace::{SpanKind, Status};

        SpanData {
            span_context: SpanContext::new(
                TraceId::from_hex("0af7651916cd43dd8448eb211c80319c").unwrap(),
                SpanId::from_hex("00f067aa0ba902b7").unwrap(),
                TraceFlags::SAMPLED,
                false,
                TraceState::default(),
            ),
            parent_span_id: SpanId::INVALID,
            parent_span_is_remote: false,
            span_kind: SpanKind::Internal,
            name: "test-span".into(),
            start_time: std::time::SystemTime::now(),
            end_time: std::time::SystemTime::now(),
            attributes: attributes.unwrap_or_default(),
            dropped_attributes_count: 0,
            events: Default::default(),
            links: Default::default(),
            status: Status::Ok,
            instrumentation_scope: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
    #[cfg(feature = "serde_json")]
    use opentelemetry::trace::{Link, SpanContext, SpanId, TraceFlags, TraceId, TraceState};

    #[test]
    fn test_format_ids() {
        for byte in 0..=u8::MAX {
            let trace_id = opentelemetry::TraceId::from_bytes([byte; 16]);
            let span_id = opentelemetry::SpanId::from_bytes([byte; 8]);
            assert_eq!(format_trace_id(trace_id), trace_id.to_string().as_bytes());
            assert_eq!(format_span_id(span_id), span_id.to_string().as_bytes());
        }

        let trace_id =
            opentelemetry::TraceId::from_hex("0af7651916cd43dd8448eb211c80319c").unwrap();
        let span_id = opentelemetry::SpanId::from_hex("00f067aa0ba902b7").unwrap();
        assert_eq!(
            format_trace_id(trace_id),
            *b"0af7651916cd43dd8448eb211c80319c"
        );
        assert_eq!(format_span_id(span_id), *b"00f067aa0ba902b7");
    }

    fn assert_timestamp_matches_chrono(datetime: DateTime<Utc>) {
        let seconds = datetime.timestamp();
        let time = if seconds >= 0 {
            SystemTime::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(seconds as u64))
        } else {
            SystemTime::UNIX_EPOCH
                .checked_sub(std::time::Duration::from_secs(seconds.unsigned_abs()))
        }
        .and_then(|time| {
            time.checked_add(std::time::Duration::from_nanos(u64::from(
                datetime.timestamp_subsec_nanos(),
            )))
        });
        // SystemTime's representable range is platform-dependent.
        if let Some(time) = time {
            assert_eq!(
                format_timestamp(time).as_ref(),
                datetime
                    .to_rfc3339_opts(SecondsFormat::AutoSi, true)
                    .as_bytes(),
                "{datetime:?}",
            );
        }
    }

    #[test]
    fn test_format_timestamp_precision_and_pre_epoch() {
        for seconds in [-1, 0, 1, 1_730_000_000] {
            for nanos in [
                0,
                1,
                10,
                100,
                1_000,
                10_000,
                100_000,
                1_000_000,
                10_000_000,
                100_000_000,
                123_000_000,
                123_456_000,
                123_456_789,
                999_999_999,
            ] {
                assert_timestamp_matches_chrono(DateTime::from_timestamp(seconds, nanos).unwrap());
            }
        }
    }

    #[test]
    fn test_format_timestamp_year_boundaries() {
        for year in [-10000, -1, 0, 1, 999, 1000, 9999, 10000] {
            for (month, day, hour, minute, second) in [(1, 1, 0, 0, 0), (12, 31, 23, 59, 59)] {
                let datetime = Utc
                    .with_ymd_and_hms(year, month, day, hour, minute, second)
                    .unwrap();
                assert_timestamp_matches_chrono(datetime);
                assert_timestamp_matches_chrono(
                    datetime + chrono::Duration::nanoseconds(999_999_999),
                );
            }
        }
        assert_timestamp_matches_chrono(DateTime::<Utc>::MIN_UTC);
        assert_timestamp_matches_chrono(DateTime::<Utc>::MAX_UTC);
    }

    #[test]
    fn test_format_timestamp_across_supported_range() {
        let min = DateTime::<Utc>::MIN_UTC.timestamp();
        let range = (DateTime::<Utc>::MAX_UTC.timestamp() - min) as u64 + 1;
        let mut state = 0x1234_5678_9abc_def0_u64;
        for _ in 0..4096 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let seconds = min + (state % range) as i64;
            let nanos = (state.rotate_left(32) % 1_000_000_000) as u32;
            assert_timestamp_matches_chrono(DateTime::from_timestamp(seconds, nanos).unwrap());
        }
    }

    #[test]
    fn test_span_kind_to_u8() {
        assert_eq!(span_kind_to_u8(&SpanKind::Internal), 0);
        assert_eq!(span_kind_to_u8(&SpanKind::Server), 1);
        assert_eq!(span_kind_to_u8(&SpanKind::Client), 2);
        assert_eq!(span_kind_to_u8(&SpanKind::Producer), 3);
        assert_eq!(span_kind_to_u8(&SpanKind::Consumer), 4);
    }

    #[cfg(feature = "serde_json")]
    #[test]
    fn test_links_to_json_with_links() {
        let links = vec![Link::new(
            SpanContext::new(
                TraceId::from_hex("0af7651916cd43dd8448eb211c80319c").unwrap(),
                SpanId::from_hex("00f067aa0ba902b7").unwrap(),
                TraceFlags::SAMPLED,
                false,
                TraceState::default(),
            ),
            vec![],
            0,
        )];
        let json = links_to_json(&links);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0]["toTraceId"], "0af7651916cd43dd8448eb211c80319c");
        assert_eq!(parsed[0]["toSpanId"], "00f067aa0ba902b7");
    }

    #[test]
    fn test_links_to_json_empty() {
        let json = links_to_json(&[]);
        assert_eq!(json, "[]");
    }
}
