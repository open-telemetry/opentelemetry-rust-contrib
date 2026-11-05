use chrono::{DateTime, Datelike, Timelike, Utc};
use opentelemetry::trace::SpanKind;
use opentelemetry::KeyValue;
use std::borrow::Cow;
use tracelogging_dynamic as tld;

#[inline]
pub(crate) fn hex_id<const N: usize>(bytes: &[u8]) -> [u8; N] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    assert_eq!(N, bytes.len() * 2);
    let mut result = [0; N];
    for (byte, pair) in bytes.iter().zip(result.chunks_exact_mut(2)) {
        pair[0] = HEX[(byte >> 4) as usize];
        pair[1] = HEX[(byte & 0x0f) as usize];
    }
    result
}

pub(crate) struct Timestamp {
    bytes: [u8; 40],
    len: usize,
}

impl AsRef<[u8]> for Timestamp {
    fn as_ref(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

#[inline]
fn write_decimal(mut value: u32, output: &mut [u8]) {
    for digit in output.iter_mut().rev() {
        *digit = b'0' + (value % 10) as u8;
        value /= 10;
    }
}

pub(crate) fn format_timestamp(time: DateTime<Utc>) -> Timestamp {
    let mut result = Timestamp {
        bytes: [0; 40],
        len: 0,
    };
    let year = time.year();
    let nano = time.nanosecond();
    // Match Chrono's signed extended years without an allocating fallback.
    if !(0..=9999).contains(&year) {
        result.bytes[0] = if year < 0 { b'-' } else { b'+' };
        result.len = 1;
    }
    let year = year.unsigned_abs();
    let width = if year < 10_000 {
        4
    } else if year < 100_000 {
        5
    } else {
        6
    };
    write_decimal(year, &mut result.bytes[result.len..result.len + width]);
    result.len += width;
    let offset = result.len;
    result.bytes[offset..offset + 15].copy_from_slice(b"-00-00T00:00:00");
    write_decimal(time.month(), &mut result.bytes[offset + 1..offset + 3]);
    write_decimal(time.day(), &mut result.bytes[offset + 4..offset + 6]);
    write_decimal(time.hour(), &mut result.bytes[offset + 7..offset + 9]);
    write_decimal(time.minute(), &mut result.bytes[offset + 10..offset + 12]);
    write_decimal(
        time.second() + nano / 1_000_000_000,
        &mut result.bytes[offset + 13..offset + 15],
    );
    result.len += 15;
    let nanos = nano % 1_000_000_000;
    if nanos != 0 {
        let (fraction, digits) = if nanos % 1_000_000 == 0 {
            (nanos / 1_000_000, 3)
        } else if nanos % 1_000 == 0 {
            (nanos / 1_000, 6)
        } else {
            (nanos, 9)
        };
        result.bytes[result.len] = b'.';
        result.len += 1;
        write_decimal(fraction, &mut result.bytes[result.len..result.len + digits]);
        result.len += digits;
    }
    result.bytes[result.len] = b'Z';
    result.len += 1;
    result
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

/// Returns the Part B field index if `key` is a well-known attribute,
/// otherwise `None`.
#[inline]
pub(crate) fn well_known_part_b_index(key: &str) -> Option<usize> {
    WELL_KNOWN_PART_B_ATTRIBUTES
        .iter()
        .position(|(otel_key, _)| *otel_key == key)
}

pub(crate) type PartBAttributes<'a> = [Option<&'a KeyValue>; WELL_KNOWN_PART_B_ATTRIBUTES.len()];

pub(crate) fn classify_attributes(attributes: &[KeyValue]) -> (PartBAttributes<'_>, usize) {
    let mut part_b = [None; WELL_KNOWN_PART_B_ATTRIBUTES.len()];
    let mut part_c_count = 0;
    for attribute in attributes {
        if let Some(index) = well_known_part_b_index(attribute.key.as_str()) {
            // Match the previous find(): first duplicate wins, and none go into Part C.
            part_b[index].get_or_insert(attribute);
        } else {
            part_c_count += 1;
        }
    }
    (part_b, part_c_count)
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
    #[cfg(feature = "serde_json")]
    use opentelemetry::trace::{Link, SpanContext, SpanId, TraceFlags, TraceId, TraceState};

    #[test]
    fn test_hex_ids_match_display() {
        use opentelemetry::trace::{SpanId, TraceId};

        for byte in 0..=u8::MAX {
            let trace = TraceId::from_bytes(std::array::from_fn(|i| byte.wrapping_add(i as u8)));
            let span = SpanId::from_bytes(std::array::from_fn(|i| byte.wrapping_sub(i as u8)));
            assert_eq!(
                hex_id::<32>(&trace.to_bytes()),
                trace.to_string().as_bytes()
            );
            assert_eq!(hex_id::<16>(&span.to_bytes()), span.to_string().as_bytes());
        }
        assert_eq!(hex_id::<32>(&[0; 16]), *b"00000000000000000000000000000000");
        assert_eq!(hex_id::<16>(&[0xff; 8]), *b"ffffffffffffffff");
    }

    #[test]
    fn test_timestamps_match_rfc3339_autosi() {
        use chrono::NaiveDate;

        let assert_matches = |time: DateTime<Utc>| {
            assert_eq!(
                format_timestamp(time).as_ref(),
                time.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
                    .as_bytes(),
                "{time:?}"
            );
        };
        assert_matches(DateTime::<Utc>::MIN_UTC);
        assert_matches(DateTime::<Utc>::MAX_UTC);
        for year in [
            -262_143, -100_000, -99_999, -10_000, -9999, -1, 0, 1, 999, 1000, 1969, 1970, 2000,
            2024, 9999, 10_000, 99_999, 100_000, 262_142,
        ] {
            for (month, day) in [(1, 1), (2, 28), (2, 29), (12, 31)] {
                let Some(date) = NaiveDate::from_ymd_opt(year, month, day) else {
                    continue;
                };
                for nano in [
                    0,
                    1,
                    100,
                    1_000,
                    1_000_000,
                    123_000_000,
                    123_456_000,
                    123_456_700,
                    999_999_999,
                    1_000_000_000,
                    1_123_456_789,
                ] {
                    for (hour, minute, second) in [(0, 0, 0), (7, 8, 9), (23, 59, 59)] {
                        if let Some(time) = date.and_hms_nano_opt(hour, minute, second, nano) {
                            assert_matches(time.and_utc());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_attribute_classification_preserves_order_and_first_duplicate() {
        let mut attributes: Vec<_> = WELL_KNOWN_PART_B_ATTRIBUTES
            .iter()
            .rev()
            .map(|(key, _)| KeyValue::new(*key, "first"))
            .collect();
        attributes.extend([
            KeyValue::new("http.request.method", "duplicate"),
            KeyValue::new("custom", 1_i64),
            KeyValue::new("custom", 2_i64),
            KeyValue::new("http.request.method.extra", "not-well-known"),
        ]);
        let (part_b, part_c_count) = classify_attributes(&attributes);
        assert_eq!(part_c_count, 3);
        for ((key, _), attribute) in WELL_KNOWN_PART_B_ATTRIBUTES.iter().zip(part_b) {
            let attribute = attribute.unwrap();
            assert_eq!(attribute.key.as_str(), *key);
            assert_eq!(attribute.value.as_str(), "first");
        }
        let (empty, count) = classify_attributes(&[]);
        assert!(empty.iter().all(Option::is_none));
        assert_eq!(count, 0);
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
