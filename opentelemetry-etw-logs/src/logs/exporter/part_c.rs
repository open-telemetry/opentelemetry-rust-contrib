use opentelemetry::logs::AnyValue;
use tracelogging_dynamic as tld;

use super::common::{EVENT_ID, EVENT_NAME_PRIMARY, EVENT_NAME_SECONDARY};

pub fn populate_part_c<'a>(
    event: &mut tld::EventBuilder,
    log_record: &'a opentelemetry_sdk::logs::SdkLogRecord,
    field_tag: u32,
) -> (Option<i64>, Option<&'a str>) {
    //populate CS PartC
    let mut event_id: Option<i64> = None;
    let mut event_name: Option<&str> = None;

    let mut cs_c_count = 0;
    for (key, value) in log_record.attributes_iter() {
        // find if we have PartC and its information
        match (key.as_str(), &value) {
            (EVENT_ID, AnyValue::Int(value)) => {
                event_id = Some(*value);
                continue;
            }
            (EVENT_NAME_PRIMARY, AnyValue::String(value)) => {
                event_name = Some(value.as_str());
                continue;
            }
            (EVENT_NAME_SECONDARY, AnyValue::String(value)) => {
                if event_name.is_none() {
                    event_name = Some(value.as_str());
                }
                continue;
            }
            _ => {
                cs_c_count += 1;
            }
        }
    }

    // If there are additional PartC attributes, add them to the event
    if cs_c_count > 0 {
        event.add_struct("PartC", cs_c_count, field_tag);

        for (key, value) in log_record.attributes_iter() {
            match (key.as_str(), &value) {
                (EVENT_ID, _) | (EVENT_NAME_PRIMARY, _) | (EVENT_NAME_SECONDARY, _) => {
                    continue;
                }
                _ => {
                    super::common::add_attribute_to_event(event, key, value);
                }
            }
        }
    }
    (event_id, event_name)
}
