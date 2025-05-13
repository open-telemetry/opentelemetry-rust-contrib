use super::etw_log_processor;
use super::ExporterOptions;

use opentelemetry_sdk::logs::LoggerProviderBuilder;

/// Extension trait for the logger provider builder to use an ETW exporter.
pub trait ETWLoggerProviderBuilderExt {
    /// Adds an ETW exporter to the logger provider builder using the given options.
    fn with_etw_exporter(self, options: ExporterOptions) -> Self;
}

impl ETWLoggerProviderBuilderExt for LoggerProviderBuilder {
    fn with_etw_exporter(self, options: ExporterOptions) -> Self {
        let reentrant_processor = etw_log_processor(options);
        self.with_log_processor(reentrant_processor)
    }
}

