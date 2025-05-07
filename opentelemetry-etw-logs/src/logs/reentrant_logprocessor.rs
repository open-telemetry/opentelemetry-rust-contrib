use std::fmt::Debug;

use opentelemetry::InstrumentationScope;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{LogBatch, LogExporter, SdkLogRecord};
use opentelemetry_sdk::Resource;

use crate::logs::exporter::*;
use crate::logs::ExporterOptions;

/// Thread-safe LogProcessor for exporting logs to ETW.
#[derive(Debug)]
pub(crate) struct ReentrantLogProcessor {
    event_exporter: ETWExporter,
}

impl ReentrantLogProcessor {
    /// Creates a new instance of the ReentrantLogProcessor using the given options.
    pub(crate) fn new(options: ExporterOptions) -> Self {
        let exporter: ETWExporter = ETWExporter::new(options);
        ReentrantLogProcessor {
            event_exporter: exporter,
        }
    }
}

/// Creates an opaque LogProcessor that can be used with the OpenTelemetry SDK.
pub fn etw_log_processor(options: ExporterOptions) -> impl opentelemetry_sdk::logs::LogProcessor {
    ReentrantLogProcessor::new(options)
}

impl opentelemetry_sdk::logs::LogProcessor for ReentrantLogProcessor {
    fn emit(&self, data: &mut SdkLogRecord, instrumentation: &InstrumentationScope) {
        let log_tuple = &[(data as &SdkLogRecord, instrumentation)];
        // TODO: How to log if export() returns Err? Maybe a metric? or eprintln?
        let _ = futures_executor::block_on(self.event_exporter.export(LogBatch::new(log_tuple)));
    }

    // This is a no-op as this processor doesn't keep anything
    // in memory to be flushed out.
    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown(&self) -> OTelSdkResult {
        self.event_exporter.shutdown()
    }

    #[cfg(feature = "spec_unstable_logs_enabled")]
    fn event_enabled(
        &self,
        level: opentelemetry::logs::Severity,
        target: &str,
        name: Option<&str>,
    ) -> bool {
        use opentelemetry_sdk::logs::LogExporter;

        self.event_exporter.event_enabled(level, target, name)
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.event_exporter.set_resource(resource);
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
        let processor = ReentrantLogProcessor::new(test_options());

        assert!(processor.shutdown().is_ok());
    }

    #[test]
    fn test_force_flush() {
        let processor = ReentrantLogProcessor::new(test_options());

        assert!(processor.force_flush().is_ok());
    }

    #[test]
    fn test_emit() {
        let processor: ReentrantLogProcessor = ReentrantLogProcessor::new(test_options());

        let mut record = SdkLoggerProvider::builder()
            .build()
            .logger("test")
            .create_log_record();
        let instrumentation = Default::default();
        processor.emit(&mut record, &instrumentation);
    }

    #[test]
    #[cfg(feature = "spec_unstable_logs_enabled")]
    fn test_event_enabled() {
        let processor = ReentrantLogProcessor::new(test_options());

        // Unit test are forced to return true as there is no ETW session listening for the event
        assert!(processor.event_enabled(opentelemetry::logs::Severity::Info, "test", Some("test")));
        assert!(processor.event_enabled(
            opentelemetry::logs::Severity::Debug,
            "test",
            Some("test")
        ));
        assert!(processor.event_enabled(
            opentelemetry::logs::Severity::Error,
            "test",
            Some("test")
        ));
    }

    fn test_options() -> ExporterOptions {
        ExporterOptions::builder("test-provider-name")
            .build()
            .unwrap()
    }
}
