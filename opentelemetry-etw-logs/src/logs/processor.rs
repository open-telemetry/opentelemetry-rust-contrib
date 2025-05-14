use std::backtrace::Backtrace;
use std::borrow::Cow;
use std::fmt::Debug;

use thiserror::Error;

use opentelemetry::otel_warn;
use opentelemetry::InstrumentationScope;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{LogBatch, LogExporter, SdkLogRecord};
use opentelemetry_sdk::Resource;

use crate::logs::exporter::*;

/// Errors that can occur while building a `Processor` instance.
#[derive(Debug)]
pub struct ProcessorBuildError {
    backtrace: Backtrace,
    kind: ProcessorBuildErrorKind,
}

impl ProcessorBuildError {
    /// Creates a new `ProcessorBuildError` with the given kind.
    pub(crate) fn new(kind: ProcessorBuildErrorKind) -> Self {
        ProcessorBuildError {
            backtrace: Backtrace::capture(),
            kind,
        }
    }

    /// True if the error is due to an empty provider name.
    pub fn is_provider_name_empty(&self) -> bool {
        matches!(self.kind, ProcessorBuildErrorKind::ProviderNameEmpty)
    }

    /// True if the error is due to a provider name that exceeds the maximum length (234 characters).
    pub fn is_provider_name_too_long(&self) -> bool {
        matches!(self.kind, ProcessorBuildErrorKind::ProviderNameTooLong)
    }

    /// True if the error is due to a provider name with invalid characters.
    pub fn is_provider_name_invalid(&self) -> bool {
        matches!(self.kind, ProcessorBuildErrorKind::ProviderNameInvalid)
    }
}

#[derive(Error, Debug, PartialEq)]
pub(crate) enum ProcessorBuildErrorKind {
    #[error("Provider name cannot be empty.")]
    ProviderNameEmpty,
    #[error("Provider name must be less than 234 characters.")]
    ProviderNameTooLong,
    #[error("Provider name must contain only ASCII alphanumeric characters, '_' or '-'.")]
    ProviderNameInvalid,
}

impl std::fmt::Display for ProcessorBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.kind)?;
        if self.backtrace.status() == std::backtrace::BacktraceStatus::Captured {
            write!(f, "\nBacktrace: {:?}", self.backtrace)?;
        }
        Ok(())
    }
}

impl std::error::Error for ProcessorBuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

/// Builder for `Processor`.
#[derive(Debug)]
pub struct ProcessorBuilder {
    options_builder: OptionsBuilder,
}

impl ProcessorBuilder {
    /// Creates a new instance of `ProcessorBuilder` with the given provider name.
    ///
    /// By default, all events will be exported to the "Log" ETW event.
    pub fn new(provider_name: impl Into<Cow<'static, str>>) -> Self {
        ProcessorBuilder {
            options_builder: Options::builder(provider_name.into()),
        }
    }

    /// Validates the options given by consuming itself and returning the `Processor` or an error.
    pub fn build(self) -> Result<Processor, ProcessorBuildError> {
        match self.options_builder.build() {
            Ok(options) => Ok(Processor::new(options)),
            Err(error) => {
                otel_warn!(name: "ETW.Processor.CreationFailed", reason = &error.to_string());
                Err(error)
            }
        }
    }
}

/// Processor for exporting logs to ETW.
/// 
/// Implements the opentelemetry_sdk::logs::LogProcessor trait, so it can be used as a log processor in the OpenTelemetry SDK.
#[derive(Debug)]
pub struct Processor {
    event_exporter: ETWExporter,
}

impl Processor {
    /// Creates a new instance of `ProcessorBuilder` with the given provider name.
    ///
    /// By default, all events will be exported to the "Log" ETW event. See `ProcessorBuilder` docs for details on how to override this behavior.
    pub fn builder(provider_name: impl Into<Cow<'static, str>>) -> ProcessorBuilder {
        ProcessorBuilder::new(provider_name)
    }

    /// Creates a new instance of the `Processor` using the given options.
    pub(crate) fn new(options: Options) -> Self {
        let exporter: ETWExporter = ETWExporter::new(options);
        Processor {
            event_exporter: exporter,
        }
    }
}

impl opentelemetry_sdk::logs::LogProcessor for Processor {
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
        let processor = Processor::new(test_options());

        assert!(processor.shutdown().is_ok());
    }

    #[test]
    fn test_force_flush() {
        let processor = Processor::new(test_options());

        assert!(processor.force_flush().is_ok());
    }

    #[test]
    fn test_emit() {
        let processor: Processor = Processor::new(test_options());

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
        let processor = Processor::new(test_options());

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

    fn test_options() -> Options {
        OptionsBuilder::new("test-provider-name").build().unwrap()
    }

    #[test]
    fn tracing_test() {
        use opentelemetry_appender_tracing::layer;
        use tracing::error;
        use tracing_subscriber::prelude::*;

        let processor = Processor::builder("provider-name").build().unwrap();
        let logger_provider = SdkLoggerProvider::builder()
            .with_log_processor(processor)
            .build();

        let layer = layer::OpenTelemetryTracingBridge::new(&logger_provider);
        let _guard = tracing_subscriber::registry().with(layer).set_default(); // Temporary subscriber active for this function

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

    #[test]
    fn test_validate_empty_name() {
        assert!(Processor::builder("")
            .build()
            .unwrap_err()
            .is_provider_name_empty());
    }

    #[test]
    fn test_validate_name_longer_than_234_chars() {
        assert!(Processor::builder("a".repeat(235))
            .build()
            .unwrap_err()
            .is_provider_name_too_long());
    }

    #[test]
    fn test_validate_name_uses_valid_chars() {
        assert!(Processor::builder("i_have_a_?_")
            .build()
            .unwrap_err()
            .is_provider_name_invalid());
    }
}
