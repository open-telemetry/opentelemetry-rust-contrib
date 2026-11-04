use crate::exporter::common;
use opentelemetry::trace::Status;
use opentelemetry::SpanId;
use opentelemetry_sdk::trace::SpanData;
use tracelogging_dynamic as tld;

/// Populates Part B of the Common Schema on the EventBuilder.
///
/// Layout (TLD struct, dynamic field count):
/// ```text
/// PartB {
///     _typeName: "Span"
///     name: str8
///     kind: u8
///     startTime: str8 (RFC3339)
///     [parentId: str8]                      // only for non-root spans
///     [links: str8 (JSON)]                  // only if present
///     TODO - statusMessage should be only conditional based on whether httpStatusCode is in or not.
///     [statusMessage: str8]                 // only if status has description
///     [<well-known attributes>: typed]      // span attributes whose keys match
///                                           // `common::WELL_KNOWN_PART_B_ATTRIBUTES`,
///                                           // emitted in slice order
///     success: bool (u8 with OutType::Boolean)
/// }
/// ```
pub(crate) fn populate_part_b(
    event: &mut tld::EventBuilder,
    span_data: &SpanData,
    field_tag: u32,
    geneva_mapping: bool,
) {
    // Calculate field count dynamically
    // Base fields: _typeName, name, kind, startTime, success = 5
    let mut field_count: u8 = 5;

    let has_parent_id = span_data.parent_span_id != SpanId::INVALID;
    if has_parent_id {
        field_count += 1;
    }

    let has_links = !span_data.links.links.is_empty();
    if has_links {
        field_count += 1;
    }

    let status_message = match &span_data.status {
        Status::Error { description } => {
            if description.is_empty() {
                None
            } else {
                Some(description.as_ref())
            }
        }
        _ => None,
    };
    if status_message.is_some() {
        field_count += 1;
    }

    // Count well-known Part B attributes that are present on this span.
    // We iterate the static slice (rather than building a map) so the
    // emitted field order is deterministic and matches the slice order.
    let well_known_count = common::WELL_KNOWN_PART_B_ATTRIBUTES
        .iter()
        .filter(|(otel_key, _)| {
            span_data
                .attributes
                .iter()
                .any(|kv| kv.key.as_str() == *otel_key)
        })
        .count() as u8;
    field_count += well_known_count;

    if !geneva_mapping {
        event.add_struct("PartB", field_count, field_tag);
        event.add_str8("_typeName", "Span", tld::OutType::Default, field_tag);
    }
    event.add_str8(
        "name",
        span_data.name.as_ref(),
        tld::OutType::Utf8,
        field_tag,
    );
    event.add_u8(
        "kind",
        common::span_kind_to_u8(&span_data.span_kind),
        tld::OutType::Default,
        field_tag,
    );
    event.add_str8(
        "startTime",
        common::format_timestamp(span_data.start_time),
        tld::OutType::Default,
        field_tag,
    );
    if has_parent_id {
        event.add_str8(
            "parentId",
            common::format_span_id(span_data.parent_span_id),
            tld::OutType::Utf8,
            field_tag,
        );
    }

    if has_links {
        let links_json = common::links_to_json(&span_data.links.links);
        event.add_str8("links", links_json.as_ref(), tld::OutType::Utf8, field_tag);
    }

    if let Some(msg) = &status_message {
        event.add_str8("statusMessage", msg, tld::OutType::Utf8, field_tag);
    }

    // Emit well-known Part B attributes using their mapped field names,
    // in the order defined by the static slice. This avoids the need for
    // a HashMap and guarantees a stable field ordering.
    for (otel_key, partb_name) in common::WELL_KNOWN_PART_B_ATTRIBUTES {
        if let Some(kv) = span_data
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == *otel_key)
        {
            common::add_attribute_to_event(event, partb_name, &kv.value);
        }
    }

    let success = !matches!(&span_data.status, Status::Error { .. });
    event.add_u8("success", success as u8, tld::OutType::Boolean, field_tag);
}

#[cfg(test)]
mod tests {
    use super::super::common::test_utils;
    use opentelemetry::trace::SpanId;
    use opentelemetry::KeyValue;

    #[test]
    fn test_export_span() {
        let exporter = test_utils::new_etw_exporter();

        let span_data = test_utils::create_test_span_data(None);
        exporter.export_span_data(&span_data);
    }

    #[test]
    fn test_export_span_with_well_known_part_b_attributes() {
        let exporter = test_utils::new_etw_exporter();

        // Mix well-known Part B attributes with regular Part C attributes.
        let span_data = test_utils::create_test_span_data(Some(vec![
            KeyValue::new("db.system", "mssql"),
            KeyValue::new("custom.attr", "value"),
            KeyValue::new("http.request.method", "GET"),
            KeyValue::new("http.response.status_code", 200_i64),
        ]));
        exporter.export_span_data(&span_data);
    }

    #[test]
    fn test_export_span_with_parent_id() {
        let exporter = test_utils::new_etw_exporter();

        let mut span_data = test_utils::create_test_span_data(None);
        span_data.parent_span_id = SpanId::from_hex("00f067aa0ba902b8").unwrap();
        exporter.export_span_data(&span_data);
    }

    #[test]
    fn test_borrowed_and_owned_error_descriptions() {
        use opentelemetry::trace::Status;
        use std::borrow::Cow;

        for geneva_mapping in [false, true] {
            for description in ["", "request failed", "request failed: \u{00e9}"] {
                let mut span_data = test_utils::create_test_span_data(None);
                span_data.status = Status::Error {
                    description: Cow::Borrowed(description),
                };
                let mut borrowed = tracelogging_dynamic::EventBuilder::new();
                super::populate_part_b(&mut borrowed, &span_data, 0, geneva_mapping);

                span_data.status = Status::Error {
                    description: Cow::Owned(description.to_owned()),
                };
                let mut owned = tracelogging_dynamic::EventBuilder::new();
                super::populate_part_b(&mut owned, &span_data, 0, geneva_mapping);
                assert_eq!(format!("{borrowed:?}"), format!("{owned:?}"));
            }
        }
    }
}
