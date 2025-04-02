use std::fmt::Debug;
use std::pin::Pin;
use std::sync::Arc;

use tracelogging_dynamic as tld;

use opentelemetry::logs::Severity;
use opentelemetry::Key;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use std::str;

mod common;
mod part_a;
mod part_b;
mod part_c;

#[derive(Default)]
struct Resource {
    pub cloud_role: Option<String>,
    pub cloud_role_instance: Option<String>,
}

pub(crate) struct ETWExporter {
    provider: Pin<Arc<tld::Provider>>,
    resource: Resource,
}

fn enabled_callback_noop(
    _source_id: &tld::Guid,
    _event_control_code: u32,
    _level: tld::Level,
    _match_any_keyword: u64,
    _match_all_keyword: u64,
    _filter_data: usize,
    _callback_context: usize,
) {
    // Unused callback.
}

impl ETWExporter {
    const KEYWORD: u64 = 1;

    pub(crate) fn new(provider_name: &str) -> Self {
        let mut options = tld::Provider::options();

        options.callback(enabled_callback_noop, 0x0);
        let provider = Arc::pin(tld::Provider::new(provider_name, &options));
        // SAFETY: tracelogging (ETW) enables an ETW callback into the provider when `register()` is called.
        // This might crash if the provider is dropped without calling unregister before.
        // This only affects static providers.
        // On dynamically created providers, the lifetime of the provider is tied to the object itself, so `unregister()` is called when dropped.
        unsafe {
            provider.as_ref().register();
        }

        ETWExporter {
            provider,
            resource: Default::default(),
        }
    }

    fn enabled(&self, level: tld::Level) -> bool {
        // On unit tests, we skip this check to be able to test the exporter as no provider is active.
        if cfg!(test) {
            return true;
        }

        self.provider.enabled(level, Self::KEYWORD)
    }

    pub(crate) fn export_log_data(
        &self,
        log_record: &opentelemetry_sdk::logs::SdkLogRecord,
        _instrumentation: &opentelemetry::InstrumentationScope,
    ) -> opentelemetry_sdk::error::OTelSdkResult {
        let otel_level = log_record.severity_number().unwrap_or(Severity::Debug);
        let level = common::convert_severity_to_level(otel_level);

        if !self.enabled(level) {
            return Ok(());
        };

        let event_tags: u32 = 0; // TBD name and event_tag values
        let field_tag: u32 = 0;
        let mut event = tld::EventBuilder::new();

        // reset
        event.reset(
            common::get_event_name(log_record),
            level,
            Self::KEYWORD,
            event_tags,
        );

        event.add_u16("__csver__", 1024, tld::OutType::Unsigned, field_tag); // 0x400 hex

        part_a::populate_part_a(&mut event, &self.resource, log_record, field_tag);

        let event_id = part_c::populate_part_c(&mut event, log_record, field_tag);

        part_b::populate_part_b(&mut event, log_record, otel_level, event_id);

        // Write event to ETW
        let result = event.write(&self.provider, None, None);

        match result {
            0 => Ok(()),
            _ => Err(OTelSdkError::InternalFailure(format!(
                "Failed to write event to ETW. ETW reason: {result}"
            ))),
        }
    }
}

impl Debug for ETWExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ETW log exporter")
    }
}

impl opentelemetry_sdk::logs::LogExporter for ETWExporter {
    async fn export(&self, batch: opentelemetry_sdk::logs::LogBatch<'_>) -> OTelSdkResult {
        if let Some((record, instrumentation)) = batch.iter().next() {
            self.export_log_data(record, instrumentation)
        } else {
            Err(OTelSdkError::InternalFailure(
                "Batch is expected to have one and only one record, but none was found".to_string(),
            ))
        }
    }

    #[cfg(feature = "spec_unstable_logs_enabled")]
    fn event_enabled(&self, level: Severity, _target: &str, _name: Option<&str>) -> bool {
        self.enabled(common::convert_severity_to_level(level))
    }

    fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
        self.resource.cloud_role = resource
            .get(&Key::from_static_str("service.name"))
            .map(|v| v.to_string());
        self.resource.cloud_role_instance = resource
            .get(&Key::from_static_str("service.instance.id"))
            .map(|v| v.to_string());
    }

    fn shutdown(&self) -> OTelSdkResult {
        let res = self.provider.as_ref().unregister();
        if res != 0 {
            return Err(OTelSdkError::InternalFailure(format!(
                "Failed to unregister provider. Win32 error: {res}"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use opentelemetry_sdk::logs::LogExporter;

    use super::*;

    #[test]
    fn test_export_log_data() {
        let record = common::test_utils::new_sdk_log_record();
        let exporter = common::test_utils::new_etw_exporter();
        let instrumentation = common::test_utils::new_instrumentation_scope();

        let result = exporter.export_log_data(&record, &instrumentation);
        assert!(result.is_ok());
    }

    #[test]
    fn test_event_name() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = common::test_utils::new_sdk_log_record();

        log_record.set_event_name("event-name");

        let exporter = common::test_utils::new_etw_exporter();
        let instrumentation = common::test_utils::new_instrumentation_scope();
        let result = exporter.export_log_data(&log_record, &instrumentation);

        assert!(result.is_ok());
    }

    #[test]
    fn test_event_resources() {
        use opentelemetry::logs::LogRecord;
        use opentelemetry::KeyValue;

        let mut log_record = common::test_utils::new_sdk_log_record();

        log_record.set_event_name("event-name");

        let mut exporter = common::test_utils::new_etw_exporter();
        exporter.set_resource(
            &opentelemetry_sdk::Resource::builder()
                .with_attributes([
                    KeyValue::new("service.name", "cloud-role-name"),
                    KeyValue::new("service.instance.id", "cloud-role-instance"),
                ])
                .build(),
        );
        let instrumentation = common::test_utils::new_instrumentation_scope();
        let result = exporter.export_log_data(&log_record, &instrumentation);

        assert!(result.is_ok());
    }

    #[test]
    fn test_debug() {
        let exporter = common::test_utils::new_etw_exporter();
        let result = format!("{:?}", exporter);
        assert_eq!(result, "ETW log exporter");
    }

    #[tokio::test]
    async fn test_export() {
        use opentelemetry_sdk::logs::LogBatch;
        use opentelemetry_sdk::logs::LogExporter;

        let log_record = common::test_utils::new_sdk_log_record();
        let instrumentation = common::test_utils::new_instrumentation_scope();

        let records = [(&log_record, &instrumentation)];
        let batch = LogBatch::new(&records);

        let exporter = common::test_utils::new_etw_exporter();
        let result = exporter.export(batch);

        assert!(result.await.is_ok());
    }

    #[test]
    fn test_callback_noop() {
        enabled_callback_noop(
            &tld::Guid::from_name("provider-name"),
            0,
            tld::Level::Verbose,
            0,
            0,
            0,
            0,
        );
    }
}
