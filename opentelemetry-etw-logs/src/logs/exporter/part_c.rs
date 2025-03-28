use opentelemetry::logs::AnyValue;
use tracelogging_dynamic as tld;

pub const EVENT_ID: &str = "event_id";

pub fn populate_part_c(
    event: &mut tld::EventBuilder,
    log_record: &opentelemetry_sdk::logs::SdkLogRecord,
    field_tag: u32,
) -> Option<i64> {
    //populate CS PartC
    let mut event_id: Option<i64> = None;

    let mut cs_c_count = 0;
    for (key, value) in log_record.attributes_iter() {
        // find if we have PartC and its information
        match (key.as_str(), &value) {
            (EVENT_ID, AnyValue::Int(value)) => {
                event_id = Some(*value);
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
                (EVENT_ID, _) => {
                    continue;
                }
                _ => {
                    super::common::add_attribute_to_event(event, key, value);
                }
            }
        }
    }
    event_id
}

#[cfg(test)]
mod tests {
    use super::super::common::test_utils;
    use super::EVENT_ID;
    use opentelemetry::logs::AnyValue;
    use opentelemetry::Key;

    #[test]
    fn test_attributes() {
        use opentelemetry::logs::LogRecord;
        use std::collections::HashMap;

        let mut log_record = test_utils::new_sdk_log_record();

        log_record.add_attribute("string", "value");
        log_record.add_attribute("int", 20);
        log_record.add_attribute("double", 1.5);
        log_record.add_attribute("boolean", true);

        log_record.add_attribute(
            "list",
            AnyValue::ListAny(Box::new(vec![AnyValue::Int(1), AnyValue::Int(2)])),
        );

        let mut map_attribute = HashMap::new();
        map_attribute.insert(Key::new("key"), AnyValue::Int(1));
        log_record.add_attribute("map", AnyValue::Map(Box::new(map_attribute)));

        log_record.add_attribute("bytes", AnyValue::Bytes(Box::new(vec![0u8, 1u8, 2u8, 3u8])));

        let exporter = test_utils::new_etw_exporter();
        let instrumentation = test_utils::new_instrumentation_scope();
        let result = exporter.export_log_data(&log_record, &instrumentation);

        assert!(result.is_ok());
    }

    #[test]
    fn test_special_attributes() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = test_utils::new_sdk_log_record();

        log_record.add_attribute(EVENT_ID, 20);

        let exporter = test_utils::new_etw_exporter();
        let instrumentation = test_utils::new_instrumentation_scope();
        let result = exporter.export_log_data(&log_record, &instrumentation);

        assert!(result.is_ok());
    }
}
