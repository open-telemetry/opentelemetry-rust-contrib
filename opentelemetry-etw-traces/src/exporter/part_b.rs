use crate::exporter::common;
use opentelemetry::trace::Status;
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
///     startTime: filetime
///     parentId: str8
///     [links: str8 (JSON)]    // only if present
///     [statusMessage: str8]   // only if status has description
///     success: bool32
/// }
/// ```
pub(crate) fn populate_part_b(event: &mut tld::EventBuilder, span_data: &SpanData, field_tag: u32) {
    // Calculate field count dynamically
    // Base fields: _typeName, name, kind, startTime, parentId, success = 6
    let mut field_count: u8 = 6;

    let has_links = !span_data.links.links.is_empty();
    if has_links {
        field_count += 1;
    }

    let status_message = match &span_data.status {
        Status::Error { description } => {
            let desc = description.to_string();
            if desc.is_empty() {
                None
            } else {
                Some(desc)
            }
        }
        _ => None,
    };
    if status_message.is_some() {
        field_count += 1;
    }

    event.add_struct("PartB", field_count, field_tag);

    event.add_str8("_typeName", "Span", tld::OutType::Default, field_tag);
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
    event.add_filetime(
        "startTime",
        tld::win_filetime_from_systemtime!(span_data.start_time),
        tld::OutType::Default,
        field_tag,
    );
    event.add_str8(
        "parentId",
        span_data.parent_span_id.to_string(),
        tld::OutType::Utf8,
        field_tag,
    );

    if has_links {
        let links_json = common::links_to_json(&span_data.links.links);
        event.add_str8("links", &links_json, tld::OutType::Utf8, field_tag);
    }

    if let Some(msg) = &status_message {
        event.add_str8("statusMessage", msg, tld::OutType::Utf8, field_tag);
    }

    let success = !matches!(&span_data.status, Status::Error { .. });
    event.add_u8("success", success as u8, tld::OutType::Boolean, field_tag);
}

#[cfg(test)]
mod tests {
    use super::super::common::test_utils;
    use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
    use opentelemetry_sdk::trace::SpanData;

    #[test]
    fn test_export_span() {
        let exporter = test_utils::new_etw_exporter();

        let span_data = create_test_span_data();
        exporter.export_span_data(&span_data);
    }

    fn create_test_span_data() -> SpanData {
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
            attributes: Vec::new(),
            dropped_attributes_count: 0,
            events: Default::default(),
            links: Default::default(),
            status: Status::Ok,
            instrumentation_scope: Default::default(),
        }
    }
}
