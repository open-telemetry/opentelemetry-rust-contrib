use eventheader::{FieldFormat, Level};
use eventheader_dynamic::{EventBuilder, EventSet, Provider};
use opentelemetry::{otel_debug, otel_info};
use opentelemetry_sdk::Resource;
use std::sync::Arc;
use std::{fmt::Debug, sync::Mutex};

use opentelemetry::{logs::AnyValue, logs::Severity, Key};
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use std::{cell::RefCell, str, time::SystemTime};

thread_local! { static EBW: RefCell<EventBuilder> = RefCell::new(EventBuilder::new());}

/// UserEventsExporter is a log exporter that exports logs in EventHeader format to user_events tracepoint.
pub(crate) struct UserEventsExporter {
    provider: Mutex<Provider>,
    name: String,
    event_sets: Vec<Arc<EventSet>>,
    cloud_role: Option<String>,
    cloud_role_instance: Option<String>,
}

// Constants for the UserEventsExporter
const EVENT_ID: &str = "event_id";
const NO_LISTENER_ERROR: i32 = 9;
const PAYLOAD_SIZE_EXCEEDED_ERROR: i32 = 34;
const CS_VERSION: u32 = 1024; // 0x400 in hex
const DEFAULT_LOG_TYPE_NAME: &str = "Log";

// Constants for EventSet registration error codes
const REGISTRATION_SUCCESS: i32 = 0;
const TRACEFS_NOT_MOUNTED_ERROR: i32 = 95;
const PERMISSION_DENIED_ERROR: i32 = 13;

/// Register event sets with the EventHeader provider
fn register_events(eventheader_provider: &mut eventheader_dynamic::Provider) -> Vec<Arc<EventSet>> {
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
            REGISTRATION_SUCCESS => {
                otel_debug!(name: "UserEvents.RegisteredTracePoint",  event_set = format!("{:?}", event_set));
            }
            TRACEFS_NOT_MOUNTED_ERROR => {
                otel_info!(name: "UserEvents.TraceFSNotMounted", event_set = format!("{:?}", event_set));
            }
            PERMISSION_DENIED_ERROR => {
                otel_info!(name: "UserEvents.PermissionDenied", event_set = format!("{:?}", event_set));
            }

            _ => {
                otel_info!(
                    name: "UserEvents.FailedToRegisterTracePoint",
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

/// Maps OpenTelemetry severity levels to EventHeader levels
const fn get_severity_level(severity: Severity) -> Level {
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

impl UserEventsExporter {
    /// Create instance of the exporter
    pub(crate) fn new(provider_name: &str) -> Self {
        let mut eventheader_provider: Provider =
            Provider::new(provider_name, &Provider::new_options());
        let event_sets = register_events(&mut eventheader_provider);
        otel_debug!(name: "UserEvents.Created", provider_name = provider_name);
        let name = eventheader_provider.name().to_string();
        UserEventsExporter {
            provider: Mutex::new(eventheader_provider),
            name,
            event_sets,
            cloud_role: None,
            cloud_role_instance: None,
        }
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
            // For unsupported types, add the key with an empty string as the value.
            // TODO: Add support for complex types with json serialization in future.
            _ => {
                eb.add_str(field_name, "", FieldFormat::Default, 0);
            }
        }
    }

    /// Gets the event name from the log record
    fn get_event_name<'a>(&self, _record: &'a opentelemetry_sdk::logs::SdkLogRecord) -> &'a str {
        // TODO: Add callback to get event name from the log record
        "Log"
    }

    /// Builds Part A of the Common Schema format
    fn build_part_a(
        &self,
        eb: &mut EventBuilder,
        log_record: &opentelemetry_sdk::logs::SdkLogRecord,
    ) {
        let mut cs_a_count = 0;
        let mut cs_a_bookmark: usize = 0;
        eb.add_struct_with_bookmark("PartA", 2, 0, &mut cs_a_bookmark);

        let event_time: SystemTime = log_record
            .timestamp()
            .or(log_record.observed_timestamp())
            .unwrap_or_else(SystemTime::now);
        let time: String =
            chrono::DateTime::to_rfc3339(&chrono::DateTime::<chrono::Utc>::from(event_time));

        cs_a_count += 1; // for event_time
                         // Add time to PartA
        eb.add_str("time", time, FieldFormat::Default, 0);

        if let Some(trace_context) = log_record.trace_context() {
            cs_a_count += 2; // for ext_dt_traceId and ext_dt_spanId
            eb.add_str(
                "ext_dt_traceId",
                trace_context.trace_id.to_string(),
                FieldFormat::Default,
                0,
            );
            eb.add_str(
                "ext_dt_spanId",
                trace_context.span_id.to_string(),
                FieldFormat::Default,
                0,
            );
        }

        if let Some(cloud_role) = &self.cloud_role {
            cs_a_count += 1;
            eb.add_str("ext_cloud_role", cloud_role, FieldFormat::Default, 0);
        }

        if let Some(cloud_role_instance) = &self.cloud_role_instance {
            cs_a_count += 1;
            eb.add_str(
                "ext_cloud_roleInstance",
                cloud_role_instance,
                FieldFormat::Default,
                0,
            );
        }

        eb.set_struct_field_count(cs_a_bookmark, cs_a_count);
    }

    pub(crate) fn export_log_data(
        &self,
        log_record: &opentelemetry_sdk::logs::SdkLogRecord,
        _instrumentation: &opentelemetry::InstrumentationScope,
    ) -> opentelemetry_sdk::error::OTelSdkResult {
        let otel_severity = log_record
            .severity_number()
            .ok_or(OTelSdkError::InternalFailure(
                "Severity number is required for user-events exporter".to_string(),
            ))?;
        let level = get_severity_level(otel_severity);

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
                let event_name = self.get_event_name(log_record);
                eb.reset(event_name, 0);

                eb.add_value("__csver__", CS_VERSION, FieldFormat::UnsignedInt, 0);

                // populate CS PartA
                self.build_part_a(&mut eb, log_record);

                //populate CS PartC
                // TODO: See if should hold on to this, and add PartB first then PartC
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
                eb.add_str("_typeName", DEFAULT_LOG_TYPE_NAME, FieldFormat::Default, 0);
                cs_b_count += 1;

                if let Some(body) = log_record.body() {
                    match body {
                        AnyValue::String(value) => {
                            eb.add_str("body", value.as_str(), FieldFormat::Default, 0);
                        }
                        AnyValue::Int(value) => {
                            eb.add_value("body", *value, FieldFormat::SignedInt, 0);
                        }
                        AnyValue::Boolean(value) => {
                            eb.add_value("body", *value, FieldFormat::Boolean, 0);
                        }
                        AnyValue::Double(value) => {
                            eb.add_value("body", *value, FieldFormat::Float, 0);
                        }
                        &_ => {
                            // TODO: Handle other types using json instead of empty string
                            eb.add_str("body", "", FieldFormat::Default, 0);
                        }
                    }
                    cs_b_count += 1;
                }

                eb.add_value("severityNumber", otel_severity as i16, FieldFormat::SignedInt, 0);
                cs_b_count += 1;

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

                let partb_name = log_record
                    .event_name()
                    .filter(|s| !s.trim().is_empty());
                if let Some(name) = partb_name {
                    eb.add_str("name", name, FieldFormat::Default, 0);
                    cs_b_count += 1;
                }

                eb.set_struct_field_count(cs_b_bookmark, cs_b_count);

                let result = eb.write(event_set, None, None);
                if result > 0 {
                    // Specially treat the case where there is no listener or payload size exceeds the limit.
                    if result == NO_LISTENER_ERROR {
                        Err(OTelSdkError::InternalFailure("Failed to write event to user_events tracepoint as there is no listener. This can occur if there was a listener when we started serializing the event, but it was removed before the event was written".to_string()))
                    } else if result == PAYLOAD_SIZE_EXCEEDED_ERROR {
                        Err(OTelSdkError::InternalFailure("Failed to write event to user_events tracepoint as total payload size exceeded 64KB limit".to_string()))
                    } else {
                        // For all other cases, return failure and include the result code.
                        Err(OTelSdkError::InternalFailure(format!(
                            "Failed to write event to user_events tracepoint with result code: {}",
                            result
                        )))
                    }
                } else {
                    Ok(())
                }
            });
            Ok(())
        } else {
            // Return success when the event is not enabled
            // as this is not an error condition.
            Ok(())
        }
    }
}

impl Debug for UserEventsExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "user_events log exporter (provider name: {})", self.name)
    }
}

impl opentelemetry_sdk::logs::LogExporter for UserEventsExporter {
    async fn export(&self, batch: opentelemetry_sdk::logs::LogBatch<'_>) -> OTelSdkResult {
        if let Some((record, instrumentation)) = batch.iter().next() {
            self.export_log_data(record, instrumentation)
        } else {
            Err(OTelSdkError::InternalFailure(
                "Batch is expected to have one and only one record, but none was found".to_string(),
            ))
        }
    }

    fn shutdown(&self) -> OTelSdkResult {
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
    fn event_enabled(&self, level: Severity, _target: &str, _name: Option<&str>) -> bool {
        // EventSets are stored in the same order as their int representation,
        // so we can use the level as index to the Vec.
        let level = get_severity_level(level);
        match self.event_sets.get(level.as_int() as usize) {
            Some(event_set) => event_set.enabled(),
            None => false,
        }
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.cloud_role = resource
            .get(&Key::from_static_str("service.name"))
            .map(|v| v.to_string());
        self.cloud_role_instance = resource
            .get(&Key::from_static_str("service.instance.id"))
            .map(|v| v.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exporter_debug() {
        let exporter = UserEventsExporter::new("test_provider");
        assert_eq!(
            format!("{:?}", exporter),
            "user_events log exporter (provider name: test_provider)"
        );
    }
}
