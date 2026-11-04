use super::common;
use opentelemetry_sdk::trace::SpanData;
use tracelogging_dynamic as tld;

/// Populates Part A of the Common Schema on the EventBuilder.
///
/// Layout (TLD struct):
/// ```text
/// PartA {
///     time: str8 (RFC3339)
///     ext_dt {
///         traceId: str8
///         spanId: str8
///     }
///     ext_cloud {
///         role: str8         // from service.name
///         roleInstance: str8 // from service.instance.id
///     }
/// }
/// ```
pub(crate) fn populate_part_a(
    event: &mut tld::EventBuilder,
    resource: &super::Resource,
    span_data: &SpanData,
    field_tag: u32,
) {
    const COUNT_TIME: u8 = 1;
    const COUNT_EXT_DT: u8 = 1;

    let field_count = COUNT_TIME + COUNT_EXT_DT + has_ext_cloud(resource);

    event.add_struct("PartA", field_count, field_tag);

    // time: use end_time as the event timestamp.
    event.add_str8(
        "time",
        common::format_timestamp(span_data.end_time),
        tld::OutType::Default,
        field_tag,
    );

    // ext_dt struct: traceId and spanId
    const EXT_DT_COUNT: u8 = 2;
    event.add_struct("ext_dt", EXT_DT_COUNT, field_tag);
    event.add_str8(
        "traceId",
        common::format_trace_id(span_data.span_context.trace_id()),
        tld::OutType::Default,
        field_tag,
    );
    event.add_str8(
        "spanId",
        common::format_span_id(span_data.span_context.span_id()),
        tld::OutType::Default,
        field_tag,
    );

    populate_resource(resource, event, field_tag);
}

#[cfg(feature = "experimental-geneva-mapping")]
pub(crate) fn populate_geneva_envelope(
    event: &mut tld::EventBuilder,
    resource: &super::Resource,
    span_data: &SpanData,
    event_name: &str,
    field_tag: u32,
) {
    event.add_str8("env_ver", "4.0", tld::OutType::Default, field_tag);
    event.add_str8("env_name", event_name, tld::OutType::Default, field_tag);
    event.add_str8(
        "env_time",
        common::format_timestamp(span_data.end_time),
        tld::OutType::Default,
        field_tag,
    );
    event.add_str8(
        "env_dt_traceId",
        common::format_trace_id(span_data.span_context.trace_id()),
        tld::OutType::Default,
        field_tag,
    );
    event.add_str8(
        "env_dt_spanId",
        common::format_span_id(span_data.span_context.span_id()),
        tld::OutType::Default,
        field_tag,
    );
    let trace_flags = span_data.span_context.trace_flags().to_u8();
    if trace_flags != 0 {
        event.add_u32(
            "env_dt_traceFlags",
            u32::from(trace_flags),
            tld::OutType::Default,
            field_tag,
        );
    }
    if let Some(role) = &resource.cloud_role {
        event.add_str8("env_cloud_role", role, tld::OutType::Default, field_tag);
    }
    if let Some(instance) = &resource.cloud_role_instance {
        event.add_str8(
            "env_cloud_roleInstance",
            instance,
            tld::OutType::Default,
            field_tag,
        );
    }
}

fn has_ext_cloud(resource: &super::Resource) -> u8 {
    (resource.cloud_role.is_some() || resource.cloud_role_instance.is_some()) as u8
}

fn populate_resource(resource: &super::Resource, event: &mut tld::EventBuilder, field_tag: u32) {
    let ext_cloud_count =
        resource.cloud_role.is_some() as u8 + resource.cloud_role_instance.is_some() as u8;
    if ext_cloud_count > 0 {
        event.add_struct("ext_cloud", ext_cloud_count, field_tag);
        if let Some(cloud_role) = &resource.cloud_role {
            event.add_str8("role", cloud_role, tld::OutType::Default, field_tag);
        }
        if let Some(cloud_role_instance) = &resource.cloud_role_instance {
            event.add_str8(
                "roleInstance",
                cloud_role_instance,
                tld::OutType::Default,
                field_tag,
            );
        }
    }
}
