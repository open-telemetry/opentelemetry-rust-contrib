use opentelemetry_sdk::trace::SpanData;
use tracelogging_dynamic as tld;

/// Populates Part A of the Common Schema on the EventBuilder.
///
/// Layout (TLD struct):
/// ```text
/// PartA {
///     time: filetime
///     ext_dt {
///         traceId: str8
///         spanId: str8
///     }
///     [role: str8]           // from service.name
///     [roleInstance: str8]   // from service.instance.id
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

    let field_count = COUNT_TIME + COUNT_EXT_DT + get_resource_count(resource);

    event.add_struct("PartA", field_count, field_tag);

    // time: use end_time as the event timestamp.
    event.add_filetime(
        "time",
        tld::win_filetime_from_systemtime!(span_data.end_time),
        tld::OutType::Default,
        field_tag,
    );

    // ext_dt struct: traceId and spanId
    const EXT_DT_COUNT: u8 = 2;
    event.add_struct("ext_dt", EXT_DT_COUNT, field_tag);
    event.add_str8(
        "traceId",
        span_data.span_context.trace_id().to_string(),
        tld::OutType::Default,
        field_tag,
    );
    event.add_str8(
        "spanId",
        span_data.span_context.span_id().to_string(),
        tld::OutType::Default,
        field_tag,
    );

    populate_resource(resource, event, field_tag);
}

fn get_resource_count(resource: &super::Resource) -> u8 {
    resource.cloud_role.is_some() as u8 + resource.cloud_role_instance.is_some() as u8
}

fn populate_resource(resource: &super::Resource, eb: &mut tld::EventBuilder, field_tag: u32) {
    if let Some(cloud_role) = &resource.cloud_role {
        eb.add_str8("role", cloud_role, tld::OutType::Default, field_tag);
    }

    if let Some(cloud_role_instance) = &resource.cloud_role_instance {
        eb.add_str8(
            "roleInstance",
            cloud_role_instance,
            tld::OutType::Default,
            field_tag,
        );
    }
}
