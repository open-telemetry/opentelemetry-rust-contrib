use crate::exporter::common;
use opentelemetry::Key;
use opentelemetry_sdk::trace::SpanData;
use tracelogging_dynamic as tld;

#[cfg(feature = "additional_promoted_attributes")]
use std::borrow::Cow;

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
    #[cfg(feature = "additional_promoted_attributes")] optional_attributes_keys: &[Cow<
        'static,
        str,
    >],
    field_tag: u32,
) {
    #[cfg_attr(not(feature = "additional_promoted_attributes"), allow(unused))]
    let mut promoted: Vec<(&Key, &opentelemetry::Value)> = Vec::new();

    // Separate attributes into promoted (Part C fields).
    #[cfg(feature = "additional_promoted_attributes")]
    {
        for kv in &span_data.attributes {
            let key_str = kv.key.as_str();

            if optional_attributes_keys.iter().any(|s| s == key_str) {
                promoted.push((&kv.key, &kv.value));
            }
        }
    }

    #[cfg(feature = "additional_promoted_attributes")]
    let promoted_count = promoted.len();
    #[cfg(not(feature = "additional_promoted_attributes"))]
    let promoted_count = 0usize;

    let resource_attr_count = resource.attributes_from_resource.len();
    let additional_span_data = 2u8; // 'attributes', 'events' as additional span data promoted as additional Part C fields.
    let total_count = promoted_count + resource_attr_count + additional_span_data as usize;

    event.add_struct("PartC", total_count.min(u8::MAX as usize) as u8, field_tag);

    // Resource attributes first
    for (key, value) in &resource.attributes_from_resource {
        common::add_attribute_to_event(event, key, value);
    }

    // Promoted span attributes as individual typed fields
    #[cfg(feature = "additional_promoted_attributes")]
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
    use opentelemetry::KeyValue;

    #[test]
    fn test_attributes_as_typed_fields() {
        let exporter = test_utils::new_etw_exporter();

        let span_data = test_utils::create_test_span_data(
            (vec![
                KeyValue::new("string_attr", "value"),
                KeyValue::new("int_attr", 42_i64),
                KeyValue::new("double_attr", 1.5_f64),
                KeyValue::new("bool_attr", true),
            ])
            .into(),
        );

        exporter.export_span_data(&span_data);
    }

    #[test]
    fn test_empty_attributes() {
        let exporter = test_utils::new_etw_exporter();

        let span_data = test_utils::create_test_span_data((vec![]).into());
        exporter.export_span_data(&span_data);
    }
}
