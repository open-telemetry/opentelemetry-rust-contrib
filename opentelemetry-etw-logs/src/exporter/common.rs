#[cfg(feature = "serde_json")]
use crate::converters::IntoJson;
use opentelemetry::{
    logs::{AnyValue, Severity},
    Key,
};
use tracelogging_dynamic as tld;

pub(crate) fn add_attribute_to_event(event: &mut tld::EventBuilder, key: &Key, value: &AnyValue) {
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
            event.add_binary(key.as_str(), b.as_slice(), tld::OutType::Default, 0);
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

pub(crate) const fn convert_severity_to_level(severity: Severity) -> tld::Level {
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

#[cfg(test)]
pub(crate) mod test_utils {
    use opentelemetry::logs::Logger;
    use opentelemetry::logs::LoggerProvider;
    use opentelemetry_sdk::logs::SdkLoggerProvider;

    use crate::exporter::options::Options;
    use crate::exporter::ETWExporter;
    use std::collections::HashSet;

    pub(crate) fn new_etw_exporter() -> ETWExporter {
        ETWExporter::new(test_options(), HashSet::new())
    }

    pub(crate) fn new_instrumentation_scope() -> opentelemetry::InstrumentationScope {
        opentelemetry::InstrumentationScope::default()
    }

    pub(crate) fn new_sdk_log_record() -> opentelemetry_sdk::logs::SdkLogRecord {
        SdkLoggerProvider::builder()
            .build()
            .logger("test")
            .create_log_record()
    }

    pub(crate) fn test_options() -> Options {
        Options::new("ContosoProvider")
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
