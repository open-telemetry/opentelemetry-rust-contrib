use std::fmt::Debug;

use opentelemetry::logs::LogResult;
use opentelemetry_sdk::export::logs::LogData;

#[cfg(feature = "logs_level_enabled")]
use opentelemetry_sdk::export::logs::LogExporter;

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
        event_name: String,
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
    fn emit(&self, data: LogData) {
        _ = self.event_exporter.export_log_data(&data);
    }

    // This is a no-op as this processor doesn't keep anything
    // in memory to be flushed out.
    fn force_flush(&self) -> LogResult<()> {
        Ok(())
    }

    // This is a no-op no special cleanup is required before
    // shutdown.
    fn shutdown(&self) -> LogResult<()> {
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
    use opentelemetry_sdk::logs::LogProcessor;

    #[test]
    fn test_shutdown() {
        let processor = ReentrantLogProcessor::new(
            "test-provider-name",
            "test-event-name".into(),
            None,
            ExporterConfig::default(),
        );

        assert!(processor.shutdown().is_ok());
    }

    #[test]
    fn test_force_flush() {
        let processor = ReentrantLogProcessor::new(
            "test-provider-name",
            "test-event-name".into(),
            None,
            ExporterConfig::default(),
        );

        assert!(processor.force_flush().is_ok());
    }

    #[test]
    fn test_emit() {
        let processor = ReentrantLogProcessor::new(
            "test-provider-name",
            "test-event-name".into(),
            None,
            ExporterConfig::default(),
        );

        let log_data = LogData {
            instrumentation: Default::default(),
            record: Default::default(),
        };

        processor.emit(log_data);
    }
}
