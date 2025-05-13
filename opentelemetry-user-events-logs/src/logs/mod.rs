mod exporter;
mod reentrant_logprocessor;

use crate::logs::exporter::UserEventsExporter;
pub use exporter::{ExportOptions, ExportOptionsBuilder};
use reentrant_logprocessor::ReentrantLogProcessor;

/// Builds an OpenTelemetry log processor backed by the User Events exporter.
///
/// This function creates a log processor that exports logs in EventHeader format
/// to user_events tracepoints. See [`ExportOptions`] for provider configuration.
///
/// # Arguments
///
/// * `options` - Exporter options, including the provider name.
///
/// # Returns
///
/// A log processor implementing [`opentelemetry_sdk::logs::LogProcessor`] that can be used with the OpenTelemetry SDK.
pub fn build_processor(
    options: ExportOptions<'_>,
) -> impl opentelemetry_sdk::logs::LogProcessor + '_ {
    let exporter = UserEventsExporter::new(options);
    ReentrantLogProcessor::new(exporter)
}
