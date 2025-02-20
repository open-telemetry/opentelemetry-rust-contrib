use std::fmt::Debug;

use opentelemetry::InstrumentationScope;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::SdkLogRecord;

#[cfg(feature = "logs_level_enabled")]
use opentelemetry_sdk::logs::LogExporter;

use crate::logs::exporter::ExporterConfig;
use crate::logs::exporter::*;

/// Thread-safe LogProcessor for exporting logs to ETW.

#[derive(Debug)]
pub struct ReentrantLogProcessor {
    event_exporter: ETWExporter,
}

impl ReentrantLogProcessor {
    /// constructor
    pub fn new(
        provider_name: &str,
        event_name: &str,
        provider_group: ProviderGroup,
        exporter_config: ExporterConfig,
    ) -> Self {
        let exporter = ETWExporter::new(provider_name, event_name, provider_group, exporter_config);
        ReentrantLogProcessor {
            event_exporter: exporter,
        }
    }
}

impl opentelemetry_sdk::logs::LogProcessor for ReentrantLogProcessor {
    fn emit(&self, data: &mut SdkLogRecord, instrumentation: &InstrumentationScope) {
        _ = self.event_exporter.export_log_data(data, instrumentation);
    }

    // This is a no-op as this processor doesn't keep anything
    // in memory to be flushed out.
    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    // This is a no-op no special cleanup is required before
    // shutdown.
    fn shutdown(&self) -> OTelSdkResult {
        Ok(())
    }

    #[cfg(feature = "logs_level_enabled")]
    fn event_enabled(
        &self,
        level: opentelemetry::logs::Severity,
        target: &str,
        name: &str,
    ) -> bool {
        self.event_exporter.event_enabled(level, target, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::logs::Logger;
    use opentelemetry::logs::LoggerProvider;
    use opentelemetry_sdk::logs::LogProcessor;
    use opentelemetry_sdk::logs::SdkLoggerProvider;

    #[test]
    fn test_shutdown() {
        let processor = ReentrantLogProcessor::new(
            "test-provider-name",
            "test-event-name",
            None,
            ExporterConfig::default(),
        );

        assert!(processor.shutdown().is_ok());
    }

    #[test]
    fn test_force_flush() {
        let processor = ReentrantLogProcessor::new(
            "test-provider-name",
            "test-event-name",
            None,
            ExporterConfig::default(),
        );

        assert!(processor.force_flush().is_ok());
    }

    #[test]
    fn test_emit() {
        let processor = ReentrantLogProcessor::new(
            "test-provider-name",
            "test-event-name",
            None,
            ExporterConfig::default(),
        );

        let mut record = SdkLoggerProvider::builder()
            .build()
            .logger("test")
            .create_log_record();
        let instrumentation = Default::default();
        processor.emit(&mut record, &instrumentation);
    }
}
