use opentelemetry_sdk::logs::TraceContext;
use std::time::SystemTime;
use tracelogging_dynamic as tld;

pub fn populate_part_a(
    event: &mut tld::EventBuilder,
    resource: &super::Resource,
    log_record: &opentelemetry_sdk::logs::SdkLogRecord,
    field_tag: u32,
) {
    if let Some(trace_context) = log_record.trace_context() {
        populate_part_a_from_context(event, resource, trace_context, field_tag);
    } else {
        populate_part_a_from_record(event, resource, field_tag);
    }

    populate_time(event, log_record, field_tag);
}

fn populate_part_a_from_record(
    event: &mut tld::EventBuilder,
    resource: &super::Resource,
    field_tag: u32,
) {
    const COUNT_TIME: u8 = 1u8;

    let field_count = COUNT_TIME + get_resource_count(resource);

    event.add_struct("PartA", field_count, field_tag);

    populate_resource(resource, event, field_tag);
}

fn populate_part_a_from_context(
    event: &mut tld::EventBuilder,
    resource: &super::Resource,
    trace_context: &TraceContext,
    field_tag: u32,
) {
    const COUNT_TIME: u8 = 1u8;
    const COUNT_EXT_DT: u8 = 1u8;
    let field_count = COUNT_TIME + COUNT_EXT_DT + get_resource_count(resource);

    event.add_struct("PartA", field_count, field_tag);

    const EXT_DT_COUNT: u8 = 2u8;
    event.add_struct("ext_dt", EXT_DT_COUNT, field_tag);
    event.add_str8(
        "traceId",
        trace_context.trace_id.to_string(),
        tld::OutType::Default,
        field_tag,
    );
    event.add_str8(
        "spanId",
        trace_context.span_id.to_string(),
        tld::OutType::Default,
        field_tag,
    );

    populate_resource(resource, event, field_tag);
}

fn get_resource_count(resource: &super::Resource) -> u8 {
    resource.cloud_role.is_some() as u8 + resource.cloud_role_instance.is_some() as u8
}

fn populate_resource(resource: &super::Resource, event: &mut tld::EventBuilder, field_tag: u32) {
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

fn populate_time(
    event: &mut tld::EventBuilder,
    log_record: &opentelemetry_sdk::logs::SdkLogRecord,
    field_tag: u32,
) {
    use chrono::{DateTime, Utc};

    let event_time: SystemTime = log_record
        .timestamp()
        .or(log_record.observed_timestamp())
        .unwrap_or_else(SystemTime::now);

    let timestamp: DateTime<Utc> = event_time.into();
    event.add_str8(
        "time",
        timestamp.to_rfc3339().as_str(),
        tld::OutType::Default,
        field_tag,
    );
}
