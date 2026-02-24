use crate::exporter::common;
use opentelemetry::Key;
use opentelemetry_sdk::trace::SpanData;
use tracelogging_dynamic as tld;

/// Known Part B attribute keys that map to dedicated Part B fields (per .NET CS40 mapping).
/// These are handled in Part B and should NOT appear in Part C.
const CS40_PART_B_KEYS: &[&str] = &[
    "db.system",
    "db.name",
    "db.statement",
    "http.method",
    "http.request.method",
    "http.url",
    "url.full",
    "http.status_code",
    "http.response.status_code",
    "messaging.system",
    "messaging.destination",
    "messaging.url",
    "otel.status_code",
    "otel.status_description",
];

/// Populates Part C of the Common Schema on the EventBuilder.
///
/// Each span attribute is serialized as a native typed TLD field (not JSON).
/// If `custom_fields` is set, only matching attributes are promoted; the rest
/// go into `env_properties` as a single JSON string.
///
/// Resource attributes specified via `with_resource_attributes()` are also
/// added as typed fields.
pub(crate) fn populate_part_c(
    eb: &mut tld::EventBuilder,
    span_data: &SpanData,
    resource: &super::Resource,
    custom_fields: Option<&Vec<String>>,
    field_tag: u32,
) {
    // Separate attributes into promoted (Part C fields) and overflow (env_properties)
    let mut promoted: Vec<(&Key, &opentelemetry::Value)> = Vec::new();
    let mut overflow: Vec<(&str, &opentelemetry::Value)> = Vec::new();

    for kv in &span_data.attributes {
        let key_str = kv.key.as_str();

        // Skip attributes already handled in Part B
        if CS40_PART_B_KEYS.contains(&key_str) {
            continue;
        }

        if let Some(cf) = custom_fields {
            if cf.iter().any(|s| s == key_str) {
                promoted.push((&kv.key, &kv.value));
            } else {
                overflow.push((key_str, &kv.value));
            }
        } else {
            // No custom_fields filter — all attributes are promoted
            promoted.push((&kv.key, &kv.value));
        }
    }

    let resource_attr_count = resource.attributes_from_resource.len();
    let has_env_properties = !overflow.is_empty();
    let total_count = promoted.len() + resource_attr_count + has_env_properties as usize;

    if total_count == 0 {
        return;
    }

    eb.add_struct("PartC", total_count.min(u8::MAX as usize) as u8, field_tag);

    // Resource attributes first
    for (key, value) in &resource.attributes_from_resource {
        common::add_attribute_to_event(eb, key, value);
    }

    // Promoted span attributes as individual typed fields
    for (key, value) in &promoted {
        common::add_attribute_to_event(eb, key, value);
    }

    // Overflow attributes as env_properties JSON
    if has_env_properties {
        let json = common::env_properties_to_json(&overflow);
        eb.add_str8("env_properties", &json, tld::OutType::Utf8, field_tag);
    }
}

#[cfg(test)]
mod tests {
    use super::super::common::test_utils;
    use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
    use opentelemetry::KeyValue;
    use opentelemetry_sdk::trace::SpanData;

    #[test]
    fn test_attributes_as_typed_fields() {
        let exporter = test_utils::new_etw_exporter();

        let span_data = create_test_span_data(vec![
            KeyValue::new("string_attr", "value"),
            KeyValue::new("int_attr", 42_i64),
            KeyValue::new("double_attr", 1.5_f64),
            KeyValue::new("bool_attr", true),
        ]);

        exporter.export_span_data(&span_data);
    }

    #[test]
    fn test_empty_attributes() {
        let exporter = test_utils::new_etw_exporter();

        let span_data = create_test_span_data(vec![]);
        exporter.export_span_data(&span_data);
    }

    fn create_test_span_data(attributes: Vec<KeyValue>) -> SpanData {
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
            attributes,
            dropped_attributes_count: 0,
            events: Default::default(),
            links: Default::default(),
            status: Status::Ok,
            instrumentation_scope: Default::default(),
        }
    }
}
