use chrono::{DateTime, Utc};
use opentelemetry::trace::SpanKind;
use opentelemetry::Key;
use opentelemetry_sdk::trace::SpanEvents;
use std::time::SystemTime;
use tracelogging_dynamic as tld;

/// Converts an OpenTelemetry `SpanKind` to a u8 value matching the OTel spec.
pub(crate) fn span_kind_to_u8(kind: &SpanKind) -> u8 {
    match kind {
        SpanKind::Internal => 1,
        SpanKind::Server => 2,
        SpanKind::Client => 3,
        SpanKind::Producer => 4,
        SpanKind::Consumer => 5,
    }
}

/// Adds an OpenTelemetry attribute value to a TLD EventBuilder as a typed field.
pub(crate) fn add_attribute_to_event(
    event: &mut tld::EventBuilder,
    key: &Key,
    value: &opentelemetry::Value,
) {
    match value {
        opentelemetry::Value::Bool(b) => {
            event.add_bool32(key.as_str(), *b as i32, tld::OutType::Default, 0);
        }
        opentelemetry::Value::I64(i) => {
            event.add_i64(key.as_str(), *i, tld::OutType::Default, 0);
        }
        opentelemetry::Value::F64(f) => {
            event.add_f64(key.as_str(), *f, tld::OutType::Default, 0);
        }
        opentelemetry::Value::String(s) => {
            event.add_str8(key.as_str(), s.as_str(), tld::OutType::Default, 0);
        }
        opentelemetry::Value::Array(arr) => {
            let json = array_to_json(arr);
            event.add_str8(key.as_str(), &json, tld::OutType::Json, 0);
        }
        _ => {
            event.add_str8(key.as_str(), "", tld::OutType::Default, 0);
        }
    }
}

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

fn array_to_json(arr: &opentelemetry::Array) -> String {
    serde_json::to_string(&array_to_value(arr)).unwrap_or_default()
}

/// Serializes span links to a JSON string (array of {toTraceId, toSpanId}).
pub(crate) fn links_to_json(links: &[opentelemetry::trace::Link]) -> String {
    if links.is_empty() {
        return "[]".to_string();
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
    serde_json::to_string(&arr).unwrap_or_default()
}

/// Builds a `serde_json::Value::Object` from key-value pairs.
fn attributes_to_value(attrs: &[opentelemetry::KeyValue]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for kv in attrs {
        let (key, value) = (kv.key.as_str(), &kv.value);
        let json_val = match value {
            opentelemetry::Value::Bool(b) => serde_json::Value::Bool(*b),
            opentelemetry::Value::I64(i) => serde_json::Value::Number((*i).into()),
            opentelemetry::Value::F64(f) => serde_json::Number::from_f64(*f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            opentelemetry::Value::String(s) => serde_json::Value::String(s.to_string()),
            opentelemetry::Value::Array(arr) => array_to_value(arr),
            _ => serde_json::Value::Null,
        };
        map.insert(key.to_owned(), json_val);
    }
    serde_json::Value::Object(map)
}

/// Serializes key-value pairs to a JSON object string for span attributes.
pub(crate) fn attributes_to_json(attrs: &[opentelemetry::KeyValue]) -> String {
    if attrs.is_empty() {
        return "{}".to_string();
    }
    serde_json::to_string(&attributes_to_value(attrs)).unwrap_or_default()
}

/// Serializes span events to a JSON object string.
pub(crate) fn events_to_json(events: &SpanEvents) -> String {
    if events.events.is_empty() {
        return "[]".to_string();
    }
    let json_value: Vec<serde_json::Value> = events
        .events
        .iter()
        .map(|event| {
            let mut map = serde_json::Map::new();
            map.insert(
                "name".to_owned(),
                serde_json::Value::String(event.name.to_string()),
            );
            map.insert(
                "timestamp".to_owned(),
                serde_json::Value::String(system_time_to_str(&event.timestamp)),
            );
            if !event.attributes.is_empty() {
                map.insert(
                    "attributes".to_owned(),
                    attributes_to_value(&event.attributes),
                );
            }
            serde_json::Value::Object(map)
        })
        .collect();
    serde_json::to_string(&json_value).unwrap_or_default()
}

pub(crate) fn system_time_to_str(time: &SystemTime) -> String {
    let datetime: DateTime<Utc> = (*time).into();
    datetime.format("%Y-%m-%d %H:%M:%S%.f").to_string()
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
    use opentelemetry::trace::{Link, SpanContext, SpanId, TraceFlags, TraceId, TraceState};
    use opentelemetry::KeyValue;
    use opentelemetry_sdk::trace::SpanEvents;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn test_span_kind_to_u8() {
        assert_eq!(span_kind_to_u8(&SpanKind::Internal), 1);
        assert_eq!(span_kind_to_u8(&SpanKind::Server), 2);
        assert_eq!(span_kind_to_u8(&SpanKind::Client), 3);
        assert_eq!(span_kind_to_u8(&SpanKind::Producer), 4);
        assert_eq!(span_kind_to_u8(&SpanKind::Consumer), 5);
    }

    #[test]
    fn test_attributes_to_json_mixed_types() {
        let attrs = vec![
            KeyValue::new("str_key", "value"),
            KeyValue::new("int_key", 42_i64),
            KeyValue::new("float_key", 3.14_f64),
            KeyValue::new("bool_key", true),
        ];
        let json = attributes_to_json(&attrs);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["str_key"], "value");
        assert_eq!(parsed["int_key"], 42);
        assert_eq!(parsed["float_key"], 3.14);
        assert_eq!(parsed["bool_key"], true);
    }

    #[test]
    fn test_attributes_to_json_empty() {
        let json = attributes_to_json(&[]);
        assert_eq!(json, "{}");
    }

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

    #[test]
    fn test_events_to_json_with_event() {
        use opentelemetry::trace::Event;

        let timestamp = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut events = SpanEvents::default();
        events.events.push(Event::new(
            "test-event",
            timestamp,
            vec![KeyValue::new("key1", "value1")],
            0,
        ));
        let json = events_to_json(&events);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0]["name"], "test-event");
        assert!(parsed[0]["timestamp"].as_str().unwrap().contains("2023"));
        assert_eq!(parsed[0]["attributes"]["key1"], "value1");
    }

    #[test]
    fn test_events_to_json_empty() {
        let events = SpanEvents::default();
        let json = events_to_json(&events);
        assert_eq!(json, "[]");
    }

    #[test]
    fn test_events_to_json_no_attributes() {
        use opentelemetry::trace::Event;

        let timestamp = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut events = SpanEvents::default();
        events
            .events
            .push(Event::new("simple-event", timestamp, vec![], 0));
        let json = events_to_json(&events);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0]["name"], "simple-event");
        assert!(parsed[0].get("attributes").is_none());
    }

    #[test]
    fn test_system_time_to_str() {
        let time = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let result = system_time_to_str(&time);
        assert!(result.starts_with("2023-11-14"));
    }
}
