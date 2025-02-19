use eventheader::{FieldFormat, Level, Opcode};
use eventheader_dynamic::EventBuilder;
use opentelemetry::{otel_debug, otel_info};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Debug;

use opentelemetry::{logs::AnyValue, logs::Severity, Key};
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use std::{cell::RefCell, str, time::SystemTime};

/// Provider group associated with the user_events exporter
pub type ProviderGroup = Option<Cow<'static, str>>;

thread_local! { static EBW: RefCell<EventBuilder> = RefCell::new(EventBuilder::new());}

/// Exporter config
#[derive(Debug)]
pub struct ExporterConfig {
    /// keyword associated with user_events name
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

    pub(crate) fn get_log_keyword_or_default(&self, name: &str) -> u64 {
        if self.keywords_map.is_empty() {
            self.default_keyword
        } else {
            self.get_log_keyword(name).unwrap_or(self.default_keyword)
        }
    }
}

/// UserEventsExporter is a log exporter that exports logs in EventHeader format to user_events tracepoint.
pub struct UserEventsExporter {
    provider: eventheader_dynamic::Provider,
    exporter_config: ExporterConfig,
}

const EVENT_ID: &str = "event_id";

impl UserEventsExporter {
    /// Create instance of the exporter
    pub fn new(
        provider_name: &str,
        _provider_group: ProviderGroup,
        exporter_config: ExporterConfig,
    ) -> Self {
        let options = eventheader_dynamic::Provider::new_options();
        let mut eventheader_provider: eventheader_dynamic::Provider =
            eventheader_dynamic::Provider::new(provider_name, &options);
        Self::register_keywords(&mut eventheader_provider, &exporter_config);
        otel_debug!(name: "UserEvents.Created", config = format!("{:?}", exporter_config), provider_name = provider_name, provider_group = format!("{:?}", options));
        UserEventsExporter {
            provider: eventheader_provider,
            exporter_config,
        }
    }

    fn register_events(eventheader_provider: &mut eventheader_dynamic::Provider, keyword: u64) {
        let levels = [
            eventheader::Level::Informational,
            eventheader::Level::Verbose,
            eventheader::Level::Warning,
            eventheader::Level::Error,
            eventheader::Level::CriticalError,
        ];

        let mut perf_command = String::from("perf record");

        for &level in levels.iter() {
            otel_debug!(
                name: "UserEvents.RegisterEvent",
                level = level.as_int(),
                keyword = keyword,
            );
            let event_set = eventheader_provider.register_set(level, keyword);
            match event_set.errno() {
                0 => {
                    otel_debug!(name: "UserEvents.RegisteredEvent",  event_set = format!("{:?}", event_set));
                }
                95 => {
                    otel_debug!(name: "UserEvents.TraceFSNotMounted", event_set = format!("{:?}", event_set));
                }
                13 => {
                    otel_debug!(name: "UserEvents.PermissionDenied", event_set = format!("{:?}", event_set));
                }

                _ => {
                    otel_debug!(
                        name: "UserEvents.FailedToRegisterEvent",
                        event_set = format!("{:?}", event_set)
                    );
                }
            }
            let event_set = eventheader_provider.find_set(level.as_int().into(), keyword);
            if let Some(set) = event_set {
                otel_debug!(name: "UserEvents.RegisteredEvent", set = format!("{:?}", set));
                // Generate and log the `perf record` command for registered event
                let event_spec = format!(
                    " -e user_events:{}_L{}K{}",
                    eventheader_provider.name(),
                    level.as_int(),
                    keyword
                );
                perf_command.push_str(&event_spec);
            } else {
                otel_debug!(
                    name: "UserEvents.FailedToRegisterEvent",
                    level = level.as_int(),
                    keyword = keyword,
                );
            }
        }
        if perf_command != "perf record" {
            println!("To listen to events, run: {}", perf_command);
            otel_debug!(name: "UserEvents.PerfCommand", command = perf_command);
        }
    }

    fn register_keywords(
        eventheader_provider: &mut eventheader_dynamic::Provider,
        exporter_config: &ExporterConfig,
    ) {
        if exporter_config.keywords_map.is_empty() {
            otel_debug!(
                name: "UserEvents.RegisterDefaultKeyword",
                default_keyword = exporter_config.default_keyword,
            );
            Self::register_events(eventheader_provider, exporter_config.default_keyword);
        }

        for keyword in exporter_config.keywords_map.values() {
            otel_debug!(
                name: "UserEvents.RegisterKeyword",
                keyword = *keyword,
            );
            Self::register_events(eventheader_provider, *keyword);
        }
    }

    fn add_attribute_to_event(&self, eb: &mut EventBuilder, (key, value): (&Key, &AnyValue)) {
        let field_name = key.as_str();
        match value.to_owned() {
            AnyValue::Boolean(b) => {
                eb.add_value(field_name, b, FieldFormat::Boolean, 0);
            }
            AnyValue::Int(i) => {
                eb.add_value(field_name, i, FieldFormat::SignedInt, 0);
            }
            AnyValue::Double(f) => {
                eb.add_value(field_name, f, FieldFormat::Float, 0);
            }
            AnyValue::String(s) => {
                eb.add_str(field_name, s.to_string(), FieldFormat::Default, 0);
            }
            _ => (),
        }
    }

    fn get_severity_level(&self, severity: Severity) -> Level {
        match severity {
            Severity::Debug
            | Severity::Debug2
            | Severity::Debug3
            | Severity::Debug4
            | Severity::Trace
            | Severity::Trace2
            | Severity::Trace3
            | Severity::Trace4 => eventheader::Level::Verbose,

            Severity::Info | Severity::Info2 | Severity::Info3 | Severity::Info4 => {
                eventheader::Level::Informational
            }

            Severity::Error | Severity::Error2 | Severity::Error3 | Severity::Error4 => {
                eventheader::Level::Error
            }

            Severity::Fatal | Severity::Fatal2 | Severity::Fatal3 | Severity::Fatal4 => {
                eventheader::Level::CriticalError
            }

            Severity::Warn | Severity::Warn2 | Severity::Warn3 | Severity::Warn4 => {
                eventheader::Level::Warning
            }
        }
    }

    #[allow(dead_code)]
    fn enabled(&self, level: u8, keyword: u64) -> bool {
        let es = self.provider.find_set(level.into(), keyword);
        match es {
            Some(x) => x.enabled(),
            _ => false,
        };
        false
    }

    pub(crate) fn export_log_data(
        &self,
        log_record: &opentelemetry_sdk::logs::SdkLogRecord,
        instrumentation: &opentelemetry::InstrumentationScope,
    ) -> opentelemetry_sdk::error::OTelSdkResult {
        //TODO - should we log (otel_debug) each event?
        let mut level: Level = Level::Invalid;
        if log_record.severity_number().is_some() {
            level = self.get_severity_level(log_record.severity_number().unwrap());
        }

        let keyword = self
            .exporter_config
            .get_log_keyword_or_default(instrumentation.name().as_ref());

        let log_es = if let Some(es) = self.provider.find_set(level.as_int().into(), keyword) {
            es
        } else {
            otel_debug!(
                name: "UserEvents.EventSetNotFound",
                level = level.as_int(),
                keyword = keyword,
            );
            return Err(OTelSdkError::InternalFailure(format!(
                "EventSet not found for level: {:?} and keyword: {}",
                level, keyword
            )));
        };
        if log_es.enabled() {
            let _res = EBW.with(|eb| {
                let mut eb = eb.borrow_mut();
                let event_tags: u32 = 0; // TBD name and event_tag values
                eb.reset(instrumentation.name().as_ref(), event_tags as u16);
                eb.opcode(Opcode::Info);

                eb.add_value("__csver__", 0x0400u16, FieldFormat::HexInt, 0);

                // populate CS PartA
                let mut cs_a_count = 0;
                let event_time: SystemTime = log_record
                    .timestamp()
                    .or(log_record.observed_timestamp())
                    .unwrap_or_else(SystemTime::now);
                cs_a_count += 1; // for event_time
                eb.add_struct("PartA", cs_a_count, 0);
                {
                    let time: String = chrono::DateTime::to_rfc3339(
                        &chrono::DateTime::<chrono::Utc>::from(event_time),
                    );
                    eb.add_str("time", time, FieldFormat::Default, 0);
                }
                //populate CS PartC
                let (mut is_event_id, mut event_id) = (false, 0);
                let (mut is_part_c_present, mut cs_c_bookmark, mut cs_c_count) = (false, 0, 0);

                for (key, value) in log_record.attributes_iter() {
                    match (key.as_str(), value) {
                        (EVENT_ID, AnyValue::Int(value)) => {
                            is_event_id = true;
                            event_id = *value;
                            continue;
                        }
                        _ => {
                            if !is_part_c_present {
                                eb.add_struct_with_bookmark("PartC", 1, 0, &mut cs_c_bookmark);
                                is_part_c_present = true;
                            }
                            self.add_attribute_to_event(&mut eb, (key, value));
                            // TODO: This is buggy and incorrectly increments the count
                            // even when the attribute is not added to PartC.
                            // This can occur when the attribute is not a primitive type.
                            cs_c_count += 1;
                        }
                    }
                }

                if is_part_c_present {
                    eb.set_struct_field_count(cs_c_bookmark, cs_c_count);
                }

                // populate CS PartB
                let mut cs_b_bookmark: usize = 0;
                let mut cs_b_count = 0;
                eb.add_struct_with_bookmark("PartB", 1, 0, &mut cs_b_bookmark);
                eb.add_str("_typeName", "Logs", FieldFormat::Default, 0);
                cs_b_count += 1;

                if log_record.body().is_some() {
                    eb.add_str(
                        "body",
                        match log_record.body().as_ref().unwrap() {
                            AnyValue::Int(value) => value.to_string(),
                            AnyValue::String(value) => value.to_string(),
                            AnyValue::Boolean(value) => value.to_string(),
                            AnyValue::Double(value) => value.to_string(),
                            AnyValue::Bytes(value) => String::from_utf8_lossy(value).to_string(),
                            AnyValue::ListAny(_value) => "".to_string(),
                            AnyValue::Map(_value) => "".to_string(),
                            &_ => "".to_string(),
                        },
                        FieldFormat::Default,
                        0,
                    );
                    cs_b_count += 1;
                }
                if level != Level::Invalid {
                    eb.add_value("severityNumber", level.as_int(), FieldFormat::SignedInt, 0);
                    cs_b_count += 1;
                }
                if log_record.severity_text().is_some() {
                    eb.add_str(
                        "severityText",
                        log_record.severity_text().as_ref().unwrap(),
                        FieldFormat::Default,
                        0,
                    );
                    cs_b_count += 1;
                }
                if is_event_id {
                    eb.add_value("eventId", event_id, FieldFormat::SignedInt, 0);
                    cs_b_count += 1;
                }
                if let Some(event_name) = log_record.event_name() {
                    eb.add_str("name", event_name, FieldFormat::Default, 0);
                    cs_b_count += 1;
                }
                eb.set_struct_field_count(cs_b_bookmark, cs_b_count);

                let result = eb.write(&log_es, None, None);
                if result > 0 {
                    // Specially log the case where there is no listener and size exceeding.
                    if result == 9 {
                        otel_debug!(name: "UserEvents.EventWriteFailed", result = result, reason = "No listener");
                    } else if result == 34 {
                        // Info level for size exceeding.
                        otel_info!(name: "UserEvents.EventWriteFailed", result = result, reason = "Total payload size exceeded 64KB limit");
                    } else {
                        // For all other cases, log the error code.
                        otel_debug!(name: "UserEvents.EventWriteFailed", result = result);
                    }
                    Err(OTelSdkError::InternalFailure(format!(
                        "Failed to write event to user_events tracepoint with result code: {}",
                        result
                    )))
                } else {
                    Ok(())
                }
            });
            Ok(())
        } else {
            otel_debug!(
                name: "UserEvents.EventSetNotEnabled",
                level = level.as_int(),
                keyword = keyword,
            );

            // Return success when the event is not enabled
            // as this is not an error condition.
            Ok(())
        }
    }
}

impl Debug for UserEventsExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("user_events log exporter")
    }
}

impl opentelemetry_sdk::logs::LogExporter for UserEventsExporter {
    #[allow(clippy::manual_async_fn)]
    fn export(
        &self,
        batch: opentelemetry_sdk::logs::LogBatch<'_>,
    ) -> impl std::future::Future<Output = OTelSdkResult> + Send {
        async move {
            for (record, instrumentation) in batch.iter() {
                let _ = self.export_log_data(record, instrumentation);
            }
            Ok(())
        }
    }

    #[cfg(feature = "spec_unstable_logs_enabled")]
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
        let es = self
            .provider
            .find_set(self.get_severity_level(level), keyword);
        match es {
            Some(x) => {
                let enabled = x.enabled();
                if !enabled {
                    otel_debug!(
                        name: "UserEvents.EventNotEnabled",
                        level = format!("{:?}",level),
                        keyword = keyword,
                    );
                }
                enabled
            }
            _ => {
                otel_debug!(
                    name: "UserEvents.EventSetNotFound",
                    level = format!("{:?}",level),
                    keyword = keyword,
                );
                false
            }
        }
    }
}
