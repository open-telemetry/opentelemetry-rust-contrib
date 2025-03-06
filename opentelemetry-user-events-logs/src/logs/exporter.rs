use eventheader::{FieldFormat, Level, Opcode};
use eventheader_dynamic::{EventBuilder, EventSet, Provider};
use opentelemetry::{otel_debug, otel_info};
use std::sync::Arc;
use std::{fmt::Debug, sync::Mutex};

use opentelemetry::{logs::AnyValue, logs::Severity, Key};
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use std::{cell::RefCell, str, time::SystemTime};

thread_local! { static EBW: RefCell<EventBuilder> = RefCell::new(EventBuilder::new());}

/// UserEventsExporter is a log exporter that exports logs in EventHeader format to user_events tracepoint.
pub struct UserEventsExporter {
    provider: Mutex<Provider>,
    event_sets: Vec<Arc<EventSet>>,
}

const EVENT_ID: &str = "event_id";

impl UserEventsExporter {
    /// Create instance of the exporter
    pub fn new(provider_name: &str) -> Self {
        let mut eventheader_provider: Provider =
            Provider::new(provider_name, &Provider::new_options());
        let event_sets = Self::register_events(&mut eventheader_provider);
        otel_debug!(name: "UserEvents.Created", provider_name = provider_name);
        UserEventsExporter {
            provider: Mutex::new(eventheader_provider),
            event_sets,
        }
    }

    fn register_events(
        eventheader_provider: &mut eventheader_dynamic::Provider,
    ) -> Vec<Arc<EventSet>> {
        let keyword: u64 = 1;
        // Levels are added in the same order as their int representation,
        // to ensure that the index of the Vec matches the int representation.
        let levels = [
            eventheader::Level::CriticalError,
            eventheader::Level::Error,
            eventheader::Level::Warning,
            eventheader::Level::Informational,
            eventheader::Level::Verbose,
        ];

        let mut event_sets = Vec::with_capacity(6);
        // Push a dummy EventSet to position 0
        // This is done so that EventSets can be retrieved using
        // level as index to the Vec.
        event_sets.push(Arc::new(EventSet::new_unregistered()));

        for &level in levels.iter() {
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

            // Always push the event set to the vector irrespective of whether
            // there is a listener or not as listeners can be added later. In
            // the event of failed registrations also, EventSet is pushed to the
            // vector, but it'll not be enabled.
            // This also ensures we can use the level as index to the Vec.
            event_sets.push(event_set);
        }
        event_sets
    }

    fn add_attribute_to_event(&self, eb: &mut EventBuilder, (key, value): (&Key, &AnyValue)) {
        let field_name = key.as_str();
        match value {
            AnyValue::Boolean(b) => {
                eb.add_value(field_name, *b, FieldFormat::Boolean, 0);
            }
            AnyValue::Int(i) => {
                eb.add_value(field_name, *i, FieldFormat::SignedInt, 0);
            }
            AnyValue::Double(f) => {
                eb.add_value(field_name, *f, FieldFormat::Float, 0);
            }
            AnyValue::String(s) => {
                eb.add_str(field_name, s.as_str(), FieldFormat::Default, 0);
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

    pub(crate) fn export_log_data(
        &self,
        log_record: &opentelemetry_sdk::logs::SdkLogRecord,
        _instrumentation: &opentelemetry::InstrumentationScope,
    ) -> opentelemetry_sdk::error::OTelSdkResult {
        let level = if let Some(otel_severity) = log_record.severity_number() {
            self.get_severity_level(otel_severity)
        } else {
            return Err(OTelSdkError::InternalFailure(
                "Severity number is required for user-events exporter".to_string(),
            ));
        };

        // EventSets are stored in the same order as their int representation,
        // so we can use the level as index to the Vec.
        let event_set = match self.event_sets.get(level.as_int() as usize) {
            Some(event_set) => event_set,
            None => {
                // This is considered Error as we cannot find the EventSet.
                // If an EventSet is found, but not enabled, it is not an error.
                return Err(OTelSdkError::InternalFailure(format!(
                    "Failed to get event set for level: {}",
                    level.as_int()
                )));
            }
        };

        if event_set.enabled() {
            let _res = EBW.with(|eb| {
                let mut eb = eb.borrow_mut();
                // EventBuilder doc suggests that event name should not be
                // reused for events with different schema. 
                // In well-behaved application, event-name should be unique
                // for each event.
                // TODO: What if the event name is not provided? "Log" is used as default.
                // TODO: Should event_tag be non-zero?
                eb.reset(log_record.event_name().unwrap_or("Log"), 0);
                eb.opcode(Opcode::Info);

                eb.add_value("__csver__", 1024, FieldFormat::UnsignedInt, 0); // 0x400 in hex

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
                eb.add_str("_typeName", "Log", FieldFormat::Default, 0);
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

                let result = eb.write(event_set, None, None);
                if result > 0 {
                    // Specially log the case where there is no listener and size exceeding.
                    if result == 9 {
                        otel_debug!(name: "UserEvents.EventWriteFailed", result = result, reason = "No listener. This can occur when there was a listener but it was removed before the event was written");
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
                level = level.as_int()
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
    async fn export(&self, batch: opentelemetry_sdk::logs::LogBatch<'_>) -> OTelSdkResult {
        for (record, instrumentation) in batch.iter() {
            let _ = self.export_log_data(record, instrumentation);
        }
        Ok(())
    }

    fn shutdown(&mut self) -> OTelSdkResult {
        // The explicit unregister() is done in shutdown()
        // as it may not be possible to unregister during Drop
        // as Loggers are typically *not* dropped.
        if let Ok(mut provider) = self.provider.lock() {
            provider.unregister();
            Ok(())
        } else {
            Err(OTelSdkError::InternalFailure(
                "Failed to acquire lock on provider".to_string(),
            ))
        }
    }

    #[cfg(feature = "spec_unstable_logs_enabled")]
    fn event_enabled(&self, level: Severity, _target: &str, _name: &str) -> bool {
        // EventSets are stored in the same order as their int representation,
        // so we can use the level as index to the Vec.
        let level = self.get_severity_level(level);
        match self.event_sets.get(level.as_int() as usize) {
            Some(event_set) => event_set.enabled(),
            None => false,
        }
    }
}
