// cargo run --example basic-logs

use opentelemetry_appender_tracing::layer;
use opentelemetry_journald_logs::JournaldLogExporter;
use opentelemetry_sdk::logs::LoggerProvider;
use tracing::info;
use tracing_subscriber::prelude::*;

fn init_logger() -> LoggerProvider {
    let exporter = JournaldLogExporter::builder()
        .with_identifier("opentelemetry-journal-exporter")
        .with_message_size_limit(4 * 1024)
        .with_attribute_prefix("OTEL")
        //.with_json_format(true) //uncomment to log in json format
        .build();

    LoggerProvider::builder()
        .with_simple_exporter(exporter)
        .build()
}

fn main() {
    let logger_provider = init_logger();
    let layer = layer::OpenTelemetryTracingBridge::new(&logger_provider);
    tracing_subscriber::registry().with(layer).init();

    info!(event_id = 1234, user_id = 5678, "my test message");
}
