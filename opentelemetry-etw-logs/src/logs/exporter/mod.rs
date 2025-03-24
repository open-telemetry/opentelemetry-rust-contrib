use std::fmt::Debug;
use std::pin::Pin;
use std::sync::Arc;

use tracelogging_dynamic as tld;

use opentelemetry::logs::Severity;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use std::str;

mod common;
mod part_a;
mod part_b;
mod part_c;

pub(crate) struct ETWExporter {
    provider: Pin<Arc<tld::Provider>>,
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

        ETWExporter { provider }
    }

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
        let level =
            self.get_severity_level(log_record.severity_number().unwrap_or(Severity::Debug));

        if !self.enabled(level) {
            return Ok(());
        };

        let event_tags: u32 = 0; // TBD name and event_tag values
        let field_tag: u32 = 0;
        let mut event = tld::EventBuilder::new();

        // reset
        event.reset(
            self.get_event_name(log_record),
            level,
            Self::KEYWORD,
            event_tags,
        );

        event.add_u16("__csver__", 0x0401u16, tld::OutType::Hex, field_tag);

        part_a::populate_part_a(&mut event, log_record, field_tag);

        let (event_id, event_name) = part_c::populate_part_c(&mut event, log_record, field_tag);

        part_b::populate_part_b(&mut event, log_record, level, event_id, event_name);

        // Write event to ETW
        let result = event.write(&self.provider, None, None);

        match result {
            0 => Ok(()),
            _ => Err(OTelSdkError::InternalFailure(format!(
                "Failed to write event to ETW. ETW reason: {result}"
            ))),
        }
    }

    fn get_event_name(&self, log_record: &opentelemetry_sdk::logs::SdkLogRecord) -> &str {
        log_record.event_name().unwrap_or("Log")
    }
}

impl Debug for ETWExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ETW log exporter")
    }
}

impl opentelemetry_sdk::logs::LogExporter for ETWExporter {
    #[allow(clippy::manual_async_fn)]
    fn export(
        &self,
        batch: opentelemetry_sdk::logs::LogBatch<'_>,
    ) -> impl std::future::Future<Output = OTelSdkResult> + Send {
        async move {
            for (log_record, instrumentation) in batch.iter() {
                let _ = self.export_log_data(log_record, instrumentation);
            }
            Ok(())
        }
    }

    #[cfg(feature = "spec_unstable_logs_enabled")]
    fn event_enabled(&self, level: Severity, _target: &str, _name: Option<&str>) -> bool {
        self.enabled(self.get_severity_level(level))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::logs::Severity;

    #[test]
    fn test_export_log_data() {
        let record = common::test_utils::new_sdk_log_record();
        let exporter = common::test_utils::new_etw_exporter();
        let instrumentation = common::test_utils::new_instrumentation_scope();

        let result = exporter.export_log_data(&record, &instrumentation);
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_severity_level() {
        let exporter = common::test_utils::new_etw_exporter();

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
