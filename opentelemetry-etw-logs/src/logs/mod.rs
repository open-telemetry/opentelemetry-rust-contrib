mod converters;
mod exporter;
mod reentrant_logprocessor;

use std::collections::HashMap;

use exporter::ExporterConfig;
use opentelemetry_sdk::logs::LoggerProviderBuilder;
use reentrant_logprocessor::ReentrantLogProcessor;

/// Extension trait for adding a ETW exporter to the logger provider builder.
pub trait ETWLoggerProviderBuilderExt {
    /// Adds an ETW exporter to the logger provider builder with the given provider name.
    ///
    /// Note that the `EventKeyword` is currently hardcoded to be `1`.
    fn with_etw_exporter(self, provider_name: &str, table_name: &str) -> Self;
}

impl ETWLoggerProviderBuilderExt for LoggerProviderBuilder {
    fn with_etw_exporter(self, provider_name: &str, table_name: &str) -> Self {
        let exporter_config = ExporterConfig {
            default_keyword: 1,
            keywords_map: HashMap::new(),
        };
        let reenterant_processor =
            ReentrantLogProcessor::new(provider_name, table_name, None, exporter_config);
        self.with_log_processor(reenterant_processor)
    }
}
