use chrono::{DateTime, Utc};
use eventheader::{FieldFormat, Level, Opcode};
use eventheader_dynamic::{EventBuilder, EventSet, Provider};
use opentelemetry::trace::SpanKind;
use opentelemetry::trace::Status;
use opentelemetry::Key;
use opentelemetry::Value;
use opentelemetry::{otel_debug, otel_info};
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::trace::SpanData;
use opentelemetry_sdk::Resource;
use std::{
    collections::HashMap,
    fmt::Debug,
    sync::{Arc, Mutex, OnceLock},
};

// Base number of fields in PartB (before adding well-known attributes)
const BASE_PARTB_FIELD_COUNT: u8 = 6;

// Well-known attributes mapping - created once at runtime
static WELL_KNOWN_ATTRIBUTES: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();

fn get_well_known_attributes() -> &'static HashMap<&'static str, &'static str> {
    WELL_KNOWN_ATTRIBUTES.get_or_init(|| {
        let mut map = HashMap::new();

        // Database attributes
        map.insert("db.system", "dbSystem");
        map.insert("db.name", "dbName");
        map.insert("db.statement", "dbStatement");

        // HTTP attributes
        map.insert("http.request.method", "httpMethod");
        map.insert("url.full", "httpUrl");
        map.insert("http.response.status_code", "httpStatusCode");

        // Messaging attributes
        map.insert("messaging.system", "messagingSystem");
        map.insert("messaging.destination", "messagingDestination");
        map.insert("messaging.url", "messagingUrl");

        map
    })
}

/// UserEventsSpanExporter exports spans in EventHeader format to user_events tracepoint.
pub(crate) struct UserEventsSpanExporter {
    provider: Mutex<Provider>,
    name: String,
    event_set: Arc<EventSet>,
    cloud_role: Option<String>,
    cloud_role_instance: Option<String>,
}

impl Debug for UserEventsSpanExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "user_events span exporter (provider: {})", self.name)
    }
}

use opentelemetry_sdk::trace::SpanExporter;

impl SpanExporter for UserEventsSpanExporter {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        if let Some(span) = batch.first() {
            self.export_span(span)
        } else {
            Err(OTelSdkError::InternalFailure(
                "Batch is expected to have one and only one record, but none was found".to_string(),
            ))
        }
    }

    fn shutdown(&mut self) -> OTelSdkResult {
        // The explicit unregister() is done in shutdown()
        // as it may not be possible to unregister during Drop
        // as `Tracers` are typically *not* dropped.
        if let Ok(mut provider) = self.provider.lock() {
            provider.unregister();
            Ok(())
        } else {
            Err(OTelSdkError::InternalFailure(
                "Failed to acquire lock on provider".to_string(),
            ))
        }
    }

    /// Set the resource for the exporter.
    fn set_resource(&mut self, resource: &Resource) {
        self.cloud_role = resource
            .get(&Key::from_static_str("service.name"))
            .map(|v| v.to_string());
        self.cloud_role_instance = resource
            .get(&Key::from_static_str("service.instance.id"))
            .map(|v| v.to_string());
    }
}

impl UserEventsSpanExporter {
    /// Create a new instance of the exporter
    pub(crate) fn new(provider_name: &str) -> Result<Self, String> {
        if provider_name.len() >= 234 {
            return Err("Provider name must be less than 234 characters.".to_string());
        }
        if !provider_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(
                "Provider name must contain only ASCII letters, digits, and '_'.".to_string(),
            );
        }

        let mut eventheader_provider = Provider::new(provider_name, &Provider::new_options());
        let keyword = 1;
        let event_set = eventheader_provider.register_set(Level::Informational, keyword);
        otel_debug!(name: "UserEvents.Created", provider_name = provider_name, event_set = format!("{:?}", event_set));
        let name = eventheader_provider.name().to_string();

        Ok(UserEventsSpanExporter {
            provider: Mutex::new(eventheader_provider),
            name,
            event_set,
            cloud_role: None,
            cloud_role_instance: None,
        })
    }

    fn add_attribute_to_event(&self, eb: &mut EventBuilder, field_name: &str, value: &Value) {
        match value {
            Value::Bool(b) => {
                eb.add_value(field_name, *b, FieldFormat::Boolean, 0);
            }
            Value::I64(i) => {
                eb.add_value(field_name, *i, FieldFormat::SignedInt, 0);
            }
            Value::F64(f) => {
                eb.add_value(field_name, *f, FieldFormat::Float, 0);
            }
            Value::String(s) => {
                eb.add_str(field_name, s.as_str(), FieldFormat::Default, 0);
            }
            // For unsupported types, add the key with an empty string as the value.
            // TODO: Add support for complex types with json serialization in future.
            _ => {
                eb.add_str(field_name, "", FieldFormat::Default, 0);
            }
        }
    }

    pub(crate) fn export_span(&self, span: &SpanData) -> OTelSdkResult {
        if self.event_set.enabled() {
            let mut eb = EventBuilder::new();
            eb.reset("Span", 0);
            eb.opcode(Opcode::Info);
            eb.add_value("__csver__", 1024, FieldFormat::UnsignedInt, 0);

            let mut cs_a_count = 3; // time, ext_dt_traceId, ext_dt_spanId
            let mut cs_a_bookmark: usize = 0;
            eb.add_struct_with_bookmark("PartA", 3, 0, &mut cs_a_bookmark);
            let datetime: DateTime<Utc> = span.end_time.into();
            eb.add_str("time", datetime.to_rfc3339(), FieldFormat::Default, 0);

            eb.add_str(
                "ext_dt_traceId",
                span.span_context.trace_id().to_string(),
                FieldFormat::Default,
                0,
            );
            eb.add_str(
                "ext_dt_spanId",
                span.span_context.span_id().to_string(),
                FieldFormat::Default,
                0,
            );

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

            // Create PartB with initial base fields. Count will be updated later
            // based on the number of well-known attributes found.
            let mut part_b_bookmark: usize = 0;
            eb.add_struct_with_bookmark("PartB", BASE_PARTB_FIELD_COUNT, 0, &mut part_b_bookmark);

            // Add base fields
            eb.add_str("_typeName", "Span", FieldFormat::Default, 0);
            eb.add_str("name", span.name.as_ref(), FieldFormat::Default, 0);
            let parent_span_id_str = if span.parent_span_id != opentelemetry::SpanId::INVALID {
                span.parent_span_id.to_string()
            } else {
                // TODO - Not to emit for root span
                "".to_string()
            };
            eb.add_str("parentId", parent_span_id_str, FieldFormat::Default, 0);
            let datetime: DateTime<Utc> = span.start_time.into();
            eb.add_str("startTime", datetime.to_rfc3339(), FieldFormat::Default, 0);
            eb.add_value(
                "success",
                matches!(span.status, Status::Ok | Status::Unset), // Check for Ok or Unset
                FieldFormat::Boolean,
                0,
            );
            eb.add_value(
                "kind",
                match span.span_kind {
                    SpanKind::Internal => 0,
                    SpanKind::Server => 1,
                    SpanKind::Client => 2,
                    SpanKind::Producer => 3,
                    SpanKind::Consumer => 4,
                },
                FieldFormat::UnsignedInt,
                0,
            );

            // Well-known attributes go into PartB.
            // Regular attributes are collected for PartC.
            // This does dual iteration (+lookup) over attributes,
            // but it is better than alternatives like allocating
            // a new vector to hold PartC attributes.
            // This could be revisited in future if performance is a concern.
            let mut partb_count_from_attributes = 0;
            let mut partc_attribute_count = 0;

            for kv in span.attributes.iter() {
                if let Some(well_known_key) = get_well_known_attributes().get(kv.key.as_str()) {
                    self.add_attribute_to_event(&mut eb, well_known_key, &kv.value);
                    partb_count_from_attributes += 1;
                } else {
                    partc_attribute_count += 1;
                }
            }

            // Update PartB field count with the number of well-known attributes found.
            eb.set_struct_field_count(
                part_b_bookmark,
                BASE_PARTB_FIELD_COUNT + partb_count_from_attributes,
            );

            // Add regular attributes to PartC if any.
            if partc_attribute_count > 0 {
                eb.add_struct("PartC", partc_attribute_count, 0);
                for kv in span.attributes.iter() {
                    if !get_well_known_attributes().contains_key(kv.key.as_str()) {
                        self.add_attribute_to_event(&mut eb, kv.key.as_str(), &kv.value);
                    }
                }
            }

            let result = eb.write(&self.event_set, None, None);
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
                return Err(OTelSdkError::InternalFailure(format!(
                    "Failed to write span event: result code {result}"
                )));
            }
        }
        Ok(())
    }
}
