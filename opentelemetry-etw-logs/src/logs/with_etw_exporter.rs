use super::new_log_processor;
use super::ExporterOptions;

use opentelemetry_sdk::logs::LoggerProviderBuilder;

/// Extension trait for the logger provider builder to use an ETW exporter.
pub trait ETWLoggerProviderBuilderExt {
    /// Adds an ETW exporter to the logger provider builder using the given options.
    fn with_etw_exporter(self, options: ExporterOptions) -> Self;
}

impl ETWLoggerProviderBuilderExt for LoggerProviderBuilder {
    fn with_etw_exporter(self, options: ExporterOptions) -> Self {
        let reentrant_processor = new_log_processor(options);
        self.with_log_processor(reentrant_processor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_appender_tracing::layer;
    use opentelemetry_sdk::logs::SdkLoggerProvider;
    use tracing::error;
    use tracing_subscriber::prelude::*;

    #[test]
    fn with_etw_exporter_trait() {
        let options = ExporterOptions::builder("provider_name").build().unwrap();
        let logger_provider = SdkLoggerProvider::builder()
            .with_etw_exporter(options)
            .build();

        let layer = layer::OpenTelemetryTracingBridge::new(&logger_provider);
        tracing_subscriber::registry().with(layer).init();

        error!(
            name: "event-name",
            event_id = 20,
            user_name = "otel user",
            user_email = "otel@opentelemetry.io"
        );

        use opentelemetry::trace::{Tracer, TracerProvider};
        let tracer_provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_sampler(opentelemetry_sdk::trace::Sampler::AlwaysOn)
            .build();
        let tracer = tracer_provider.tracer("test-tracer");

        tracer.in_span("test-span", |_cx| {
            // logging is done inside span context.
            error!(
                name: "event-name",
                event_id = 20,
                user_name = "otel user",
                user_email = "otel@opentelemetry.io"
            );
        });
    }
}
