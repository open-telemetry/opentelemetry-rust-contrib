use std::collections::HashMap;

use exporter::{ExporterConfig, UserEventsExporter};
use opentelemetry_sdk::logs::LoggerProviderBuilder;
use reentrant_logprocessor::ReentrantLogProcessor;

mod exporter;
mod reentrant_logprocessor;

///  Extension trait for adding a user event exporter to the logger provider builder.
pub trait UserEventsLoggerProviderBuilderExt {
    /// Adds a user event exporter to the logger provider builder,
    /// with the given provider name.
    fn with_user_event_exporter(self, provider_name: &str) -> Self;
}
impl UserEventsLoggerProviderBuilderExt for LoggerProviderBuilder {
    fn with_user_event_exporter(self, provider_name: &str) -> Self {
        let exporter_config = ExporterConfig {
            default_keyword: 1,
            keywords_map: HashMap::new(),
        };
        let exporter = UserEventsExporter::new(provider_name, None, exporter_config);
        let reenterant_processor = ReentrantLogProcessor::new(exporter);
        self.with_log_processor(reenterant_processor)
    }
}
