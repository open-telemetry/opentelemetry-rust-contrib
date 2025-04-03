use opentelemetry::{logs::Severity, Key};
use tracelogging_dynamic as tld;

pub fn populate_part_b(
    event: &mut tld::EventBuilder,
    log_record: &opentelemetry_sdk::logs::SdkLogRecord,
    level: Severity,
    event_id: Option<i64>,
) {
    // Count fields in PartB
    const COUNT_TYPE_NAME: u8 = 1u8;
    const COUNT_SEVERITY_NUMBER: u8 = 1u8;

    let field_count = COUNT_TYPE_NAME
        + COUNT_SEVERITY_NUMBER
        + log_record.body().is_some() as u8
        + log_record.severity_text().is_some() as u8
        + event_id.is_some() as u8
        + log_record.event_name().is_some() as u8;

    // Create PartB struct
    event.add_struct("PartB", field_count, 0);

    // Fill fields of PartB struct
    event.add_str8("_typeName", "Log", tld::OutType::Default, 0);

    if let Some(body) = log_record.body() {
        super::common::add_attribute_to_event(event, &Key::new("body"), body);
    }

    event.add_i16("severityNumber", level as i16, tld::OutType::Default, 0);

    if let Some(severity_text) = &log_record.severity_text() {
        event.add_str8("severityText", severity_text, tld::OutType::Default, 0);
    }

    if let Some(event_id) = event_id {
        event.add_i64("eventId", event_id, tld::OutType::Default, 0);
    }

    if let Some(event_name) = log_record.event_name() {
        event.add_str8("name", event_name, tld::OutType::Default, 0);
    }
}

#[cfg(test)]
mod test {
    use super::super::common::test_utils;

    #[test]
    fn test_body() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = test_utils::new_sdk_log_record();

        log_record.set_body("body".into());

        let exporter = test_utils::new_etw_exporter();
        let instrumentation = test_utils::new_instrumentation_scope();
        let result = exporter.export_log_data(&log_record, &instrumentation);

        assert!(result.is_ok());
    }
}
