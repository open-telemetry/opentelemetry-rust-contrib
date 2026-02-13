use opentelemetry::trace::SpanKind;
use opentelemetry::Key;
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

fn array_to_json(arr: &opentelemetry::Array) -> String {
    use opentelemetry::Array;
    match arr {
        Array::Bool(v) => serde_json::to_string(v).unwrap_or_default(),
        Array::I64(v) => serde_json::to_string(v).unwrap_or_default(),
        Array::F64(v) => serde_json::to_string(v).unwrap_or_default(),
        Array::String(v) => {
            let strs: Vec<&str> = v.iter().map(|s| s.as_str()).collect();
            serde_json::to_string(&strs).unwrap_or_default()
        }
        _ => String::new(),
    }
}

/// Serializes span links to a JSON string (array of {toTraceId, toSpanId}).
pub(crate) fn links_to_json(links: &[opentelemetry::trace::Link]) -> String {
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

/// Serializes key-value pairs to a JSON object string for env_properties.
pub(crate) fn env_properties_to_json(attrs: &[(&str, &opentelemetry::Value)]) -> String {
    let mut map = serde_json::Map::new();
    for (key, value) in attrs {
        let json_val = match value {
            opentelemetry::Value::Bool(b) => serde_json::Value::Bool(*b),
            opentelemetry::Value::I64(i) => serde_json::Value::Number((*i).into()),
            opentelemetry::Value::F64(f) => serde_json::Number::from_f64(*f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            opentelemetry::Value::String(s) => serde_json::Value::String(s.to_string()),
            opentelemetry::Value::Array(arr) => {
                serde_json::from_str(&array_to_json(arr)).unwrap_or(serde_json::Value::Null)
            }
            _ => serde_json::Value::Null,
        };
        map.insert(key.to_string(), json_val);
    }
    serde_json::to_string(&map).unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod test_utils {
    use super::super::options::Options;
    use super::super::ETWExporter;

    pub(crate) fn new_etw_exporter() -> ETWExporter {
        ETWExporter::new(test_options())
    }

    pub(crate) fn test_options() -> Options {
        Options::new("ContosoProvider")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_span_kind_to_u8() {
        assert_eq!(span_kind_to_u8(&SpanKind::Internal), 1);
        assert_eq!(span_kind_to_u8(&SpanKind::Server), 2);
        assert_eq!(span_kind_to_u8(&SpanKind::Client), 3);
        assert_eq!(span_kind_to_u8(&SpanKind::Producer), 4);
        assert_eq!(span_kind_to_u8(&SpanKind::Consumer), 5);
    }
}
