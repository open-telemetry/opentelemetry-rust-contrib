use crate::exporter::common;
use opentelemetry_sdk::trace::SpanData;
use tracelogging_dynamic as tld;

/// Populates Part C of the Common Schema on the EventBuilder.
///
/// Span attributes are added as typed fields, except for well-known
/// attributes (see [`common::WELL_KNOWN_PART_B_ATTRIBUTES`]) which are
/// emitted as Part B fields instead.
/// Resource attributes specified via `with_resource_attributes()` are also
/// added as typed fields.
pub(crate) fn populate_part_c(
    event: &mut tld::EventBuilder,
    span_data: &SpanData,
    resource: &super::Resource,
    field_tag: u32,
) {
    let resource_attr_count = resource.attributes_from_resource.len();
    let span_partc_attr_count = span_data
        .attributes
        .iter()
        .filter(|kv| common::well_known_part_b_field(kv.key.as_str()).is_none())
        .count();
    let part_c_count = resource_attr_count + span_partc_attr_count;

    event.add_struct(
        "PartC",
        part_c_count.try_into().unwrap_or(u8::MAX),
        field_tag,
    );

    // Resource attributes first
    for (key, value) in &resource.attributes_from_resource {
        common::add_attribute_to_event(event, key.as_str(), value);
    }

    // Span attributes (excluding well-known Part B attributes)
    for kv in span_data.attributes.iter() {
        if common::well_known_part_b_field(kv.key.as_str()).is_some() {
            continue;
        }
        common::add_attribute_to_event(event, kv.key.as_str(), &kv.value);
    }
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
