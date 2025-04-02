use crate::logs::reentrant_logprocessor::{validate_provider_name, ReentrantLogProcessor};
use opentelemetry::otel_warn;
use opentelemetry_sdk::logs::LoggerProviderBuilder;

/// Extension trait for adding a ETW exporter to the logger provider builder.
pub trait ETWLoggerProviderBuilderExt {
    /// Adds an ETW exporter to the logger provider builder with the given provider name and event_name.
    fn with_etw_exporter(self, provider_name: &str) -> Self;
}

impl ETWLoggerProviderBuilderExt for LoggerProviderBuilder {
    fn with_etw_exporter(self, provider_name: &str) -> Self {
        if let Err(error) = validate_provider_name(provider_name) {
            otel_warn!(name: "ETW.Exporter.CreationFailed", reason = &error);
            self
        } else {
            let reentrant_processor = ReentrantLogProcessor::new(provider_name);
            self.with_log_processor(reentrant_processor)
        }
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
        let logger_provider = SdkLoggerProvider::builder()
            .with_etw_exporter("provider-name")
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
