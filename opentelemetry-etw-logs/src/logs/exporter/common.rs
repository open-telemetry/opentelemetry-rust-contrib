#[cfg(feature = "serde_json")]
use crate::logs::converters::IntoJson;
use opentelemetry::{
    logs::{AnyValue, Severity},
    Key,
};
use tracelogging_dynamic as tld;

pub fn add_attribute_to_event(event: &mut tld::EventBuilder, key: &Key, value: &AnyValue) {
    match value {
        AnyValue::Boolean(b) => {
            event.add_bool32(key.as_str(), *b as i32, tld::OutType::Default, 0);
        }
        AnyValue::Int(i) => {
            event.add_i64(key.as_str(), *i, tld::OutType::Default, 0);
        }
        AnyValue::Double(f) => {
            event.add_f64(key.as_str(), *f, tld::OutType::Default, 0);
        }
        AnyValue::String(s) => {
            event.add_str8(key.as_str(), s.as_str(), tld::OutType::Default, 0);
        }
        AnyValue::Bytes(b) => {
            event.add_binaryc(key.as_str(), b.as_slice(), tld::OutType::Default, 0);
        }
        #[cfg(feature = "serde_json")]
        AnyValue::ListAny(_) => {
            event.add_str8(
                key.as_str(),
                value.as_json_value().to_string(),
                tld::OutType::Json,
                0,
            );
        }
        #[cfg(feature = "serde_json")]
        AnyValue::Map(_) => {
            event.add_str8(
                key.as_str(),
                value.as_json_value().to_string(),
                tld::OutType::Json,
                0,
            );
        }
        &_ => {
            // For unsupported types, add the key with an empty string as the value.
            event.add_str8(key.as_str(), "", tld::OutType::Default, 0);
        }
    }
}

pub const fn convert_severity_to_level(severity: Severity) -> tld::Level {
    match severity {
        Severity::Debug
        | Severity::Debug2
        | Severity::Debug3
        | Severity::Debug4
        | Severity::Trace
        | Severity::Trace2
        | Severity::Trace3
        | Severity::Trace4 => tld::Level::Verbose,

        Severity::Info | Severity::Info2 | Severity::Info3 | Severity::Info4 => {
            tld::Level::Informational
        }

        Severity::Error | Severity::Error2 | Severity::Error3 | Severity::Error4 => {
            tld::Level::Error
        }

        Severity::Fatal | Severity::Fatal2 | Severity::Fatal3 | Severity::Fatal4 => {
            tld::Level::Critical
        }

        Severity::Warn | Severity::Warn2 | Severity::Warn3 | Severity::Warn4 => tld::Level::Warning,
    }
}

pub fn get_event_name(log_record: &opentelemetry_sdk::logs::SdkLogRecord) -> &str {
    log_record.event_name().unwrap_or("Log")
}

#[cfg(test)]
pub mod test_utils {
    use opentelemetry::logs::Logger;
    use opentelemetry::logs::LoggerProvider;
    use opentelemetry_sdk::logs::SdkLoggerProvider;

    use super::super::ETWExporter;

    pub fn new_etw_exporter() -> ETWExporter {
        ETWExporter::new("test-provider-name")
    }

    pub fn new_instrumentation_scope() -> opentelemetry::InstrumentationScope {
        opentelemetry::InstrumentationScope::default()
    }

    pub fn new_sdk_log_record() -> opentelemetry_sdk::logs::SdkLogRecord {
        SdkLoggerProvider::builder()
            .build()
            .logger("test")
            .create_log_record()
    }
}

#[test]
fn test_get_severity_level() {
    let result = convert_severity_to_level(Severity::Debug);
    assert_eq!(result, tld::Level::Verbose);

    let result = convert_severity_to_level(Severity::Info);
    assert_eq!(result, tld::Level::Informational);

    let result = convert_severity_to_level(Severity::Error);
    assert_eq!(result, tld::Level::Error);

    let result = convert_severity_to_level(Severity::Fatal);
    assert_eq!(result, tld::Level::Critical);

    let result = convert_severity_to_level(Severity::Warn);
    assert_eq!(result, tld::Level::Warning);
}

#[test]
fn test_get_event_name() {
    use opentelemetry::logs::LogRecord;

    let mut log_record = test_utils::new_sdk_log_record();

    let result = get_event_name(&log_record);
    assert_eq!(result, "Log");

    log_record.set_event_name("event-name");
    let result = get_event_name(&log_record);
    assert_eq!(result, "event-name");
}
