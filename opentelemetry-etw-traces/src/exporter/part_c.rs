use crate::exporter::common;
use opentelemetry::Key;
use opentelemetry_sdk::trace::SpanData;
use tracelogging_dynamic as tld;

/// Populates Part C of the Common Schema on the EventBuilder.
///
/// Each span attribute is serialized as a native typed TLD field (not JSON).
/// If `optional_attributes_keys` is set, only matching attributes are promoted; the rest
/// go into `attributes` JSON representation.
///
/// Resource attributes specified via `with_resource_attributes()` are also
/// added as typed fields.
pub(crate) fn populate_part_c(
    event: &mut tld::EventBuilder,
    span_data: &SpanData,
    resource: &super::Resource,
    optional_attributes_keys: Option<&Vec<String>>,
    field_tag: u32,
) {
    // Separate attributes into promoted (Part C fields).
    let mut promoted: Vec<(&Key, &opentelemetry::Value)> = Vec::new();

    for kv in &span_data.attributes {
        let key_str = kv.key.as_str();

        if let Some(cf) = optional_attributes_keys {
            if cf.iter().any(|s| s == key_str) {
                promoted.push((&kv.key, &kv.value));
            }
        }
    }

    let resource_attr_count = resource.attributes_from_resource.len();
    let additional_span_data = 2u8; // 'attributes', 'events' as additional span data promoted as additional Part C fields.
    let total_count = promoted.len() + resource_attr_count + additional_span_data as usize;

    if total_count == 0 {
        return;
    }

    event.add_struct("PartC", total_count.min(u8::MAX as usize) as u8, field_tag);

    // Resource attributes first
    for (key, value) in &resource.attributes_from_resource {
        common::add_attribute_to_event(event, key, value);
    }

    // Promoted span attributes as individual typed fields
    for (key, value) in &promoted {
        common::add_attribute_to_event(event, key, value);
    }

    // 'attributes' and 'events' as JSON string fields
    event.add_str8(
        "attributes",
        common::attributes_to_json(&span_data.attributes),
        tld::OutType::Utf8,
        field_tag,
    );
    event.add_str8(
        "events",
        common::events_to_json(&span_data.events),
        tld::OutType::Utf8,
        field_tag,
    );
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
