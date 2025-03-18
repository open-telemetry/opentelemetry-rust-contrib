use chrono::{DateTime, Utc};
use eventheader::{FieldFormat, Level, Opcode};
use eventheader_dynamic::{EventBuilder, EventSet, Provider};
use opentelemetry::trace::SpanKind;
use opentelemetry::trace::Status;
use opentelemetry::{KeyValue, Value};
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::trace::SpanData;
use std::{
    fmt::Debug,
    sync::{Arc, Mutex},
};

/// UserEventsSpanExporter exports spans in EventHeader format to user_events tracepoint.
pub(crate) struct UserEventsSpanExporter {
    provider: Mutex<Provider>,
    name: String,
    event_set: Arc<EventSet>,
}

impl Debug for UserEventsSpanExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "user_events span exporter (provider: {})", self.name)
    }
}

use futures_util::future::BoxFuture;
use opentelemetry_sdk::trace::SpanExporter;

impl SpanExporter for UserEventsSpanExporter {
    fn export(&mut self, batch: Vec<SpanData>) -> BoxFuture<'static, OTelSdkResult> {
        // Using try_for_each is safe here as we do not expect the batch to have more than one span.
        let result = batch.iter().try_for_each(|span| self.export_span(span));

        Box::pin(async move { result })
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
        let name = eventheader_provider.name().to_string();

        Ok(UserEventsSpanExporter {
            provider: Mutex::new(eventheader_provider),
            name,
            event_set,
        })
    }

    fn add_attribute_to_event(&self, eb: &mut EventBuilder, kv: &KeyValue) {
        let field_name = kv.key.as_str();
        match &kv.value {
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
            _ => (),
        }
    }

    pub(crate) fn export_span(&self, span: &SpanData) -> OTelSdkResult {
        if self.event_set.enabled() {
            let mut eb = EventBuilder::new();
            eb.reset("Span", 0);
            eb.opcode(Opcode::Info);
            eb.add_value("__csver__", 1024, FieldFormat::UnsignedInt, 0);

            eb.add_struct("PartA", 2, 0); // time, ext_dt
            let datetime: DateTime<Utc> = span.end_time.into();
            eb.add_str("time", datetime.to_rfc3339(), FieldFormat::Default, 0);
            eb.add_struct("ext_dt", 2, 0);
            eb.add_str(
                "traceId",
                span.span_context.trace_id().to_string(),
                FieldFormat::Default,
                0,
            );
            eb.add_str(
                "spanId",
                span.span_context.span_id().to_string(),
                FieldFormat::Default,
                0,
            );

            eb.add_struct("PartB", 5, 0);
            eb.add_str("_typeName", "Span", FieldFormat::Default, 0);
            eb.add_str("name", span.name.as_ref(), FieldFormat::Default, 0);
            eb.add_str(
                "parentId",
                span.parent_span_id.to_string(),
                FieldFormat::Default,
                0,
            );
            let datetime: DateTime<Utc> = span.start_time.into();
            eb.add_str("startTime", datetime.to_rfc3339(), FieldFormat::Default, 0);
            eb.add_value(
                "success",
                matches!(span.status, Status::Ok | Status::Unset), // Check for Ok or Unset
                FieldFormat::Boolean,
                0,
            );
            eb.add_str(
                "kind",
                match span.span_kind {
                    SpanKind::Internal => "internal",
                    SpanKind::Server => "server",
                    SpanKind::Client => "client",
                    SpanKind::Producer => "producer",
                    SpanKind::Consumer => "consumer",
                },
                FieldFormat::Default,
                0,
            );
            eb.add_struct("PartC", span.attributes.len() as u8, 0);
            for kv in span.attributes.iter() {
                self.add_attribute_to_event(&mut eb, kv);
            }

            let result = eb.write(&*self.event_set, None, None);
            if result > 0 {
                return Err(OTelSdkError::InternalFailure(format!(
                    "Failed to write span event: result code {}",
                    result
                )));
            }
        }
        Ok(())
    }
}
