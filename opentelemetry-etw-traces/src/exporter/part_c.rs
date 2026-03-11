use crate::exporter::common;
use opentelemetry_sdk::trace::SpanData;
use tracelogging_dynamic as tld;

/// Populates Part C of the Common Schema on the EventBuilder.
///
/// Span attributes are added as typed fields.
/// Resource attributes specified via `with_resource_attributes()` are also
/// added as typed fields.
pub(crate) fn populate_part_c(
    event: &mut tld::EventBuilder,
    span_data: &SpanData,
    resource: &super::Resource,
    field_tag: u32,
) {
    let resource_attr_count = resource.attributes_from_resource.len();
    let additional_span_data = 2u8; // 'attributes', 'events'
    let total_count = resource_attr_count + additional_span_data as usize;

    event.add_struct("PartC", total_count.min(u8::MAX as usize) as u8, field_tag);

    // Resource attributes first
    for (key, value) in &resource.attributes_from_resource {
        common::add_attribute_to_event(event, key, value);
    }

    // Span attributes
    for kv in span_data.attributes.iter() {
        common::add_attribute_to_event(event, &kv.key, &kv.value);
    }

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
