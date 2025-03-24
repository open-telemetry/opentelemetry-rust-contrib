use crate::logs::converters::IntoJson;
use opentelemetry::{logs::AnyValue, Key};
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
        AnyValue::ListAny(l) => {
            event.add_str8(
                key.as_str(),
                l.as_json_value().to_string(),
                tld::OutType::Json,
                0,
            );
        }
        AnyValue::Map(m) => {
            event.add_str8(
                key.as_str(),
                m.as_json_value().to_string(),
                tld::OutType::Json,
                0,
            );
        }
        &_ => {}
    }
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
