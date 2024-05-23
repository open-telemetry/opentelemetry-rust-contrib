use async_trait::async_trait;
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Debug;
use std::pin::Pin;
use std::sync::Arc;
use tracelogging::win_filetime_from_systemtime;
use tracelogging_dynamic as tld;

use opentelemetry::{
    logs::{AnyValue, Severity},
    Key,
};
use std::{str, time::SystemTime};

use crate::logs::converters::IntoJson;

/// Provider group associated with the ETW exporter
pub type ProviderGroup = Option<Cow<'static, str>>;

// thread_local! { static EBW: RefCell<EventBuilder> = RefCell::new(EventBuilder::new());}

/// Exporter config
#[derive(Debug)]
pub struct ExporterConfig {
    /// keyword associated with ETW name
    /// These should be mapped to logger_name as of now.
    pub keywords_map: HashMap<String, u64>,
    /// default keyword if map is not defined.
    pub default_keyword: u64,
}

impl Default for ExporterConfig {
    fn default() -> Self {
        ExporterConfig {
            keywords_map: HashMap::new(),
            default_keyword: 1,
        }
    }
}

impl ExporterConfig {
    pub(crate) fn get_log_keyword(&self, name: &str) -> Option<u64> {
        self.keywords_map.get(name).copied()
    }

    pub(crate) fn get_log_keyword_or_default(&self, name: &str) -> Option<u64> {
        if self.keywords_map.is_empty() {
            Some(self.default_keyword)
        } else {
            self.get_log_keyword(name)
        }
    }
}
pub(crate) struct ETWExporter {
    provider: Pin<Arc<tld::Provider>>,
    exporter_config: ExporterConfig,
    event_name: String,
}

const EVENT_ID: &str = "event_id";
const EVENT_NAME_PRIMARY: &str = "event_name";
const EVENT_NAME_SECONDARY: &str = "name";

// TODO: Implement callback
fn enabled_callback(
    _source_id: &tld::Guid,
    _event_control_code: u32,
    _level: tld::Level,
    _match_any_keyword: u64,
    _match_all_keyword: u64,
    _filter_data: usize,
    _callback_context: usize,
) {
}

//TBD - How to configure provider name and provider group
impl ETWExporter {
    pub(crate) fn new(
        provider_name: &str,
        event_name: String,
        _provider_group: ProviderGroup,
        exporter_config: ExporterConfig,
    ) -> Self {
        let mut options = tld::Provider::options();
        // TODO: Implement callback
        options.callback(enabled_callback, 0x0);
        let provider = Arc::pin(tld::Provider::new(provider_name, &options));
        // SAFETY: tracelogging (ETW) enables an ETW callback into the provider when `register()` is called.
        // This might crash if the provider is dropped without calling unregister before.
        // This only affects static providers.
        // On dynamically created providers, the lifetime of the provider is tied to the object itself, so `unregister()` is called when dropped.
        unsafe {
            provider.as_ref().register();
        }
        // TODO: enable keywords on callback
        // Self::register_keywords(&mut provider, &exporter_config);
        ETWExporter {
            provider,
            exporter_config,
            event_name,
        }
    }

    // TODO: enable keywords on callback
    // fn register_events(provider: &mut tld::Provider, keyword: u64) {
    //     let levels = [
    //         tld::Level::Verbose,
    //         tld::Level::Informational,
    //         tld::Level::Warning,
    //         tld::Level::Error,
    //         tld::Level::Critical,
    //         tld::Level::LogAlways,
    //     ];

    //     for &level in levels.iter() {
    //         // provider.register_set(level, keyword);
    //     }
    // }

    // fn register_keywords(provider: &mut tld::Provider, exporter_config: &ExporterConfig) {
    //     if exporter_config.keywords_map.is_empty() {
    //         println!(
    //             "Register default keyword {}",
    //             exporter_config.default_keyword
    //         );
    //         Self::register_events(provider, exporter_config.default_keyword);
    //     }

    //     for keyword in exporter_config.keywords_map.values() {
    //         Self::register_events(provider, *keyword);
    //     }
    // }

    fn get_severity_level(&self, severity: Severity) -> tld::Level {
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

            Severity::Warn | Severity::Warn2 | Severity::Warn3 | Severity::Warn4 => {
                tld::Level::Warning
            }
        }
    }

    #[allow(dead_code)]
    fn enabled(&self, level: u8, keyword: u64) -> bool {
        // TODO: Use internal enabled check. Meaning of enable differs from OpenTelemetry and ETW.
        // OpenTelemetry wants to know if level+keyword combination is enabled for the Provider.
        // ETW tells if level+keyword combination is being actively listened. Not all systems actively
        // listens for ETW events, but they do it on samples.
        // This may be fixed by applying the OpenTelemetry logic in the callback function.
        self.provider.enabled(level.into(), keyword)
    }

    pub(crate) fn export_log_data(
        &self,
        log_data: &opentelemetry_sdk::export::logs::LogData,
    ) -> opentelemetry_sdk::export::logs::ExportResult {
        let level =
            self.get_severity_level(log_data.record.severity_number.unwrap_or(Severity::Debug));

        let keyword = match self
            .exporter_config
            .get_log_keyword_or_default(log_data.instrumentation.name.as_ref())
        {
            Some(keyword) => keyword,
            _ => return Ok(()),
        };

        if !self.provider.enabled(level.as_int().into(), keyword) {
            return Ok(());
        };

        let event_tags: u32 = 0; // TBD name and event_tag values
        let field_tag: u32 = 0;
        let mut event = tld::EventBuilder::new();

        // reset
        event.reset(&self.event_name, level, keyword, event_tags);

        event.add_u16("__csver__", 0x0401u16, tld::OutType::Hex, field_tag);

        self.populate_part_a(&mut event, log_data, field_tag);

        let (event_id, event_name) = self.populate_part_c(&mut event, log_data, field_tag);

        self.populate_part_b(&mut event, log_data, level, event_id, event_name);

        // Write event to ETW
        let result = event.write(&self.provider, None, None);

        match result {
            0 => Ok(()),
            _ => Err(format!("Failed to write event to ETW. ETW reason: {result}").into()),
        }
    }

    fn populate_part_a(
        &self,
        event: &mut tld::EventBuilder,
        log_data: &opentelemetry_sdk::export::logs::LogData,
        field_tag: u32,
    ) {
        let event_time: SystemTime = log_data
            .record
            .timestamp
            .or(log_data.record.observed_timestamp)
            .unwrap_or_else(SystemTime::now);

        const COUNT_TIME: u8 = 1u8;
        const PART_A_COUNT: u8 = COUNT_TIME;
        event.add_struct("PartA", PART_A_COUNT, field_tag);
        {
            let timestamp = win_filetime_from_systemtime!(event_time);
            event.add_filetime("time", timestamp, tld::OutType::Default, field_tag);
        }
    }

    fn populate_part_b(
        &self,
        event: &mut tld::EventBuilder,
        log_data: &opentelemetry_sdk::export::logs::LogData,
        level: tld::Level,
        event_id: Option<i64>,
        event_name: Option<&str>,
    ) {
        // Count fields in PartB
        const COUNT_TYPE_NAME: u8 = 1u8;
        const COUNT_SEVERITY_NUMBER: u8 = 1u8;

        let field_count = COUNT_TYPE_NAME
            + COUNT_SEVERITY_NUMBER
            + log_data.record.body.is_some() as u8
            + log_data.record.severity_text.is_some() as u8
            + event_id.is_some() as u8
            + event_name.is_some() as u8;

        // Create PartB struct
        event.add_struct("PartB", field_count, 0);

        // Fill fields of PartB struct
        event.add_str8("_typeName", "Logs", tld::OutType::Default, 0);

        if let Some(body) = log_data.record.body.clone() {
            add_attribute_to_event(event, &Key::new("body"), &body);
        }

        event.add_u8("severityNumber", level.as_int(), tld::OutType::Default, 0);

        if let Some(severity_text) = &log_data.record.severity_text {
            event.add_str8(
                "severityText",
                severity_text.as_ref(),
                tld::OutType::Default,
                0,
            );
        }

        if let Some(event_id) = event_id {
            event.add_i64("eventId", event_id, tld::OutType::Default, 0);
        }

        if let Some(event_name) = event_name {
            event.add_str8("name", event_name, tld::OutType::Default, 0);
        }
    }

    fn populate_part_c<'a>(
        &'a self,
        event: &mut tld::EventBuilder,
        log_data: &'a opentelemetry_sdk::export::logs::LogData,
        field_tag: u32,
    ) -> (Option<i64>, Option<&str>) {
        //populate CS PartC
        let mut event_id: Option<i64> = None;
        let mut event_name: Option<&str> = None;

        if let Some(attr_list) = &log_data.record.attributes {
            let mut cs_c_count = 0;

            // find if we have PartC and its information
            for (key, value) in attr_list.iter() {
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

            if cs_c_count > 0 {
                event.add_struct("PartC", cs_c_count, field_tag);

                for (key, value) in attr_list.iter() {
                    match (key.as_str(), &value) {
                        (EVENT_ID, _) | (EVENT_NAME_PRIMARY, _) | (EVENT_NAME_SECONDARY, _) => {
                            continue;
                        }
                        _ => {
                            add_attribute_to_event(event, key, value);
                        }
                    }
                }
            }
        }

        (event_id, event_name)
    }
}

impl Debug for ETWExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ETW log exporter")
    }
}

#[async_trait]
impl opentelemetry_sdk::export::logs::LogExporter for ETWExporter {
    async fn export(
        &mut self,
        batch: Vec<opentelemetry_sdk::export::logs::LogData>,
    ) -> opentelemetry::logs::LogResult<()> {
        for log_data in batch {
            let _ = self.export_log_data(&log_data);
        }
        Ok(())
    }

    #[cfg(feature = "logs_level_enabled")]
    fn event_enabled(&self, level: Severity, _target: &str, name: &str) -> bool {
        let (found, keyword) = if self.exporter_config.keywords_map.is_empty() {
            (true, self.exporter_config.default_keyword)
        } else {
            // TBD - target is not used as of now for comparison.
            match self.exporter_config.get_log_keyword(name) {
                Some(x) => (true, x),
                _ => (false, 0),
            }
        };
        if !found {
            return false;
        }
        self.provider
            .enabled(self.get_severity_level(level), keyword)
    }
}

fn add_attribute_to_event(event: &mut tld::EventBuilder, key: &Key, value: &AnyValue) {
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
                &l.as_json_value().to_string(),
                tld::OutType::Json,
                0,
            );
        }
        AnyValue::Map(m) => {
            event.add_str8(
                key.as_str(),
                &m.as_json_value().to_string(),
                tld::OutType::Json,
                0,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::logs::Severity;
    use opentelemetry_sdk::export::logs::LogData;

    #[test]
    fn test_export_log_data() {
        let exporter = ETWExporter::new(
            "test-provider-name",
            "test-event-name".to_string(),
            None,
            ExporterConfig::default(),
        );

        let log_data = LogData {
            instrumentation: Default::default(),
            record: Default::default(),
        };

        let result = exporter.export_log_data(&log_data);
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_severity_level() {
        let exporter = ETWExporter::new(
            "test-provider-name",
            "test-event-name".to_string(),
            None,
            ExporterConfig::default(),
        );

        let result = exporter.get_severity_level(Severity::Debug);
        assert_eq!(result, tld::Level::Verbose);

        let result = exporter.get_severity_level(Severity::Info);
        assert_eq!(result, tld::Level::Informational);

        let result = exporter.get_severity_level(Severity::Error);
        assert_eq!(result, tld::Level::Error);

        let result = exporter.get_severity_level(Severity::Fatal);
        assert_eq!(result, tld::Level::Critical);

        let result = exporter.get_severity_level(Severity::Warn);
        assert_eq!(result, tld::Level::Warning);
    }
}
