use opentelemetry_sdk::logs::TraceContext;
use std::time::SystemTime;
use tracelogging::win_filetime_from_systemtime;
use tracelogging_dynamic as tld;

pub fn populate_part_a(
    event: &mut tld::EventBuilder,
    log_record: &opentelemetry_sdk::logs::SdkLogRecord,
    field_tag: u32,
) {
    if let Some(trace_context) = log_record.trace_context() {
        populate_part_a_from_context(event, trace_context, field_tag);
    } else {
        populate_part_a_from_record(event, field_tag);
    }

    populate_time(event, log_record, field_tag);
}

fn populate_part_a_from_record(event: &mut tld::EventBuilder, field_tag: u32) {
    const COUNT_TIME: u8 = 1u8;
    const PART_A_COUNT: u8 = COUNT_TIME;
    event.add_struct("PartA", PART_A_COUNT, field_tag);
}

fn populate_part_a_from_context(
    event: &mut tld::EventBuilder,
    trace_context: &TraceContext,
    field_tag: u32,
) {
    const COUNT_TIME: u8 = 1u8;
    const COUNT_EXT_DT: u8 = 1u8;
    const PART_A_COUNT: u8 = COUNT_TIME + COUNT_EXT_DT;

    event.add_struct("PartA", PART_A_COUNT, field_tag);

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
}

fn populate_time(
    event: &mut tld::EventBuilder,
    log_record: &opentelemetry_sdk::logs::SdkLogRecord,
    field_tag: u32,
) {
    let event_time: SystemTime = log_record
        .timestamp()
        .or(log_record.observed_timestamp())
        .unwrap_or_else(SystemTime::now);

    let timestamp = win_filetime_from_systemtime!(event_time);
    event.add_filetime("time", timestamp, tld::OutType::Default, field_tag);
}
