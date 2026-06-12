use opentelemetry::trace::SpanKind;
use std::borrow::Cow;
use tracelogging_dynamic as tld;

/// Well-known OpenTelemetry semantic-convention attributes that map to
/// Common Schema Part B field names. Attributes whose key matches one of
/// these entries are emitted as Part B fields (using the mapped name)
/// instead of Part C.
///
/// Stored as a static slice (rather than a `HashMap`) to guarantee a
/// stable, deterministic ordering of fields in the emitted ETW event.
pub(crate) const WELL_KNOWN_PART_B_ATTRIBUTES: &[(&str, &str)] = &[
    // Database
    ("db.system", "dbSystem"),
    ("db.name", "dbName"),
    ("db.statement", "dbStatement"),
    // HTTP
    ("http.request.method", "httpMethod"),
    ("url.full", "httpUrl"),
    ("http.response.status_code", "httpStatusCode"),
    // Messaging
    ("messaging.system", "messagingSystem"),
    ("messaging.destination", "messagingDestination"),
    ("messaging.url", "messagingUrl"),
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
    #[cfg(feature = "serde_json")]
    use opentelemetry::trace::{Link, SpanContext, SpanId, TraceFlags, TraceId, TraceState};

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
