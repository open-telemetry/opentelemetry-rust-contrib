use opentelemetry::InstrumentationScope;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{LogBatch, LogExporter, SdkLogRecord};
use opentelemetry_sdk::Resource;
use std::error::Error;
use std::fmt::Debug;

use crate::exporter::*;

/// Processes and exports logs to ETW.
///
/// This processor exports logs without synchronization.
/// It is specifically designed for the ETW exporter, where
/// the underlying exporter is safe under concurrent calls.
///
/// Implements the opentelemetry_sdk::logs::LogProcessor trait, so it can be used as a log processor in the OpenTelemetry SDK.
#[derive(Debug)]
pub struct Processor {
    event_exporter: ETWExporter,
}

impl Processor {
    /// Creates a new instance of [`ProcessorBuilder`] with the given provider name.
    ///
    /// The provider name must be cross-compatible with UserEvents (Linux) and must conform to the following requirements:
    ///
    /// - non-empty,
    /// - less than 234 characters long, and
    /// - contain only ASCII alphanumeric characters or underscores (`_`).
    ///
    /// At the same time, it is recommended to use a provider name that is:
    /// - short
    /// - human-readable
    /// - unique
    /// - describing the application or service that is generating the logs
    ///
    /// By default, all events will be exported to the "Log" ETW event. See [`ProcessorBuilder`] for details on how to configure a different behavior.
    pub fn builder(provider_name: &str) -> ProcessorBuilder {
        ProcessorBuilder::new(provider_name)
    }

    /// Creates a new instance of [`ProcessorBuilder`] with the given provider name conforming to ETW requirements.
    ///
    /// Cross-compatibility with UserEvents (Linux) is not guaranteed. Following are the explicit requirements:
    ///
    /// - non-empty,
    /// - less than 234 characters long, and
    /// - contain only ASCII alphanumeric characters, underscores (`_`) or hyphens (`-`).
    ///
    /// At the same time, it is recommended to use a provider name that is:
    /// - short
    /// - human-readable
    /// - unique
    /// - describing the application or service that is generating the logs
    ///
    /// By default, all events will be exported to the "Log" ETW event. See [`ProcessorBuilder`] for details on how to configure a different behavior.
    pub fn builder_etw_compat_only(provider_name: &str) -> ProcessorBuilder {
        ProcessorBuilder::new_etw_compat_only(provider_name)
    }

    /// Creates a new instance of the [`Processor`] using the given options.
    pub(crate) fn new(options: Options) -> Self {
        let exporter: ETWExporter = ETWExporter::new(options);
        Processor {
            event_exporter: exporter,
        }
    }
}

#[derive(Debug, Copy, Clone)]
enum ProviderNameCompatMode {
    /// Cross-compatible with UserEvents (Linux).
    CrossCompat,
    /// ETW only compatible.
    EtwCompatOnly,
}

impl opentelemetry_sdk::logs::LogProcessor for Processor {
    fn emit(&self, data: &mut SdkLogRecord, instrumentation: &InstrumentationScope) {
        let log_tuple = &[(data as &SdkLogRecord, instrumentation)];
        // TODO: How to log if export() returns Err? Maybe a metric? or eprintln?
        let _ = futures_executor::block_on(self.event_exporter.export(LogBatch::new(log_tuple)));
    }

    // Nothing to flush as this processor does not buffer
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

/// Builder for configuring and constructing a [`Processor`].
#[derive(Debug)]
pub struct ProcessorBuilder {
    options: Options,
    provider_name_compat_mode: ProviderNameCompatMode,
}

impl ProcessorBuilder {
    /// Creates a new instance of [`ProcessorBuilder`] with the given provider name.
    ///
    /// The provider name must contain only ASCII alphanumeric characters or '_'.
    ///
    /// By default, all events will be exported to the "Log" ETW event.
    pub(crate) fn new(provider_name: &str) -> Self {
        ProcessorBuilder {
            options: Options::new(provider_name.to_string()),
            provider_name_compat_mode: ProviderNameCompatMode::CrossCompat,
        }
    }

    /// Creates a new instance of [`ProcessorBuilder`] with the given provider name.
    ///
    /// The provider name must contain only ASCII alphanumeric characters, '_' or '-'.
    ///
    /// By default, all events will be exported to the "Log" ETW event.
    pub(crate) fn new_etw_compat_only(provider_name: &str) -> Self {
        ProcessorBuilder {
            options: Options::new(provider_name.to_string()),
            provider_name_compat_mode: ProviderNameCompatMode::EtwCompatOnly,
        }
    }

    /// Sets a user-defined callback that returns the ETW event name, using the the [`SdkLogRecord`] as input.
    ///
    /// The resulting name must be a valid CommonSchema 4.0 TraceLoggingDynamic event name. Otherwise,
    /// the default "Log" ETW event name will be used.
    #[cfg(feature = "logs_unstable_etw_event_name_from_callback")]
    pub fn etw_event_name_from_callback(
        mut self,
        callback: impl Fn(&SdkLogRecord) -> &'static str + Send + Sync + 'static,
    ) -> Self {
        self.options = self.options.etw_event_name_from_callback(callback);
        self
    }

    /// Builds the processor with given options, returning `Error` if it fails.
    pub fn build(self) -> Result<Processor, Box<dyn Error>> {
        self.validate()?;

        Ok(Processor::new(self.options))
    }

    fn validate(&self) -> Result<(), Box<dyn Error>> {
        validate_provider_name(self.options.provider_name(), self.provider_name_compat_mode)?;
        Ok(())
    }
}

fn validate_provider_name(
    provider_name: &str,
    compat_mode: ProviderNameCompatMode,
) -> Result<(), Box<dyn Error>> {
    if provider_name.is_empty() {
        return Err("Provider name must not be empty.".into());
    }
    if provider_name.len() >= 234 {
        return Err("Provider name must be less than 234 characters long.".into());
    }

    match compat_mode {
        ProviderNameCompatMode::CrossCompat => {
            if !provider_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err(
                    "Provider name must contain only ASCII alphanumeric characters or '_'.".into(),
                );
            }
        }
        ProviderNameCompatMode::EtwCompatOnly => {
            if !provider_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(
                    "Provider name must contain only ASCII alphanumeric characters, '_' or '-'."
                        .into(),
                );
            }
        }
    }

    Ok(())
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
        Options::new("test_provider_name")
    }

    #[test]
    fn tracing_integration_test() {
        use opentelemetry_appender_tracing::layer;
        use tracing::error;
        use tracing_subscriber::prelude::*;

        #[allow(
            unused_mut
            //, reason = "We require this to be mut if the 'logs_unstable_etw_event_name_from_callback' feature is enabled"
        )]
        let mut processor_builder = Processor::builder("provider_name");

        #[cfg(feature = "logs_unstable_etw_event_name_from_callback")]
        {
            processor_builder = processor_builder.etw_event_name_from_callback(|_| "CustomEvent")
        }

        let processor = processor_builder.build().unwrap();
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
        assert_eq!(
            Processor::builder("").build().unwrap_err().to_string(),
            "Provider name must not be empty."
        );
    }

    #[test]
    fn test_validate_name_longer_than_234_chars() {
        assert_eq!(
            Processor::builder("a".repeat(235).as_str())
                .build()
                .unwrap_err()
                .to_string(),
            "Provider name must be less than 234 characters long."
        );
    }

    #[test]
    fn test_validate_name_uses_valid_chars() {
        assert_eq!(
            Processor::builder("i_have_a_?_")
                .build()
                .unwrap_err()
                .to_string(),
            "Provider name must contain only ASCII alphanumeric characters or '_'."
        );
    }

    #[test]
    fn test_validate_cross_compat_name_not_using_hyphens() {
        assert_eq!(
            Processor::builder("i_have_a_-_")
                .build()
                .unwrap_err()
                .to_string(),
            "Provider name must contain only ASCII alphanumeric characters or '_'."
        );
    }

    #[test]
    fn test_validate_etw_compat_name_using_hyphens() {
        assert!(Processor::builder_etw_compat_only("i_have_a_-_")
            .build()
            .is_ok());
    }

    #[test]
    fn test_validate_provider_name_cross_compat() {
        let compat_mode: ProviderNameCompatMode = ProviderNameCompatMode::CrossCompat;

        let result = validate_provider_name("valid_provider_name", compat_mode);
        assert!(result.is_ok());

        let result = validate_provider_name("", compat_mode);
        assert!(result.is_err());

        let result = validate_provider_name("a".repeat(235).as_str(), compat_mode);
        assert!(result.is_err());

        let result = validate_provider_name("i_have_a_-_", compat_mode);
        assert!(result.is_err());

        let result = validate_provider_name("_?_", compat_mode);
        assert!(result.is_err());

        let result = validate_provider_name("abcdefghijklmnopqrstuvwxyz", compat_mode);
        assert!(result.is_ok());

        let result = validate_provider_name("ABCDEFGHIJKLMNOPQRSTUVWXYZ", compat_mode);
        assert!(result.is_ok());

        let result = validate_provider_name("1234567890", compat_mode);
        assert!(result.is_ok());

        let result = validate_provider_name(
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890_",
            compat_mode,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_provider_name_etw_compat() {
        let compat_mode: ProviderNameCompatMode = ProviderNameCompatMode::EtwCompatOnly;

        let result = validate_provider_name("valid_provider_name", compat_mode);
        assert!(result.is_ok());

        let result = validate_provider_name("", compat_mode);
        assert!(result.is_err());

        let result = validate_provider_name("a".repeat(235).as_str(), compat_mode);
        assert!(result.is_err());

        let result = validate_provider_name("i_have_a_-_", compat_mode);
        assert!(result.is_ok());

        let result = validate_provider_name("_?_", compat_mode);
        assert!(result.is_err());

        let result = validate_provider_name("abcdefghijklmnopqrstuvwxyz", compat_mode);
        assert!(result.is_ok());

        let result = validate_provider_name("ABCDEFGHIJKLMNOPQRSTUVWXYZ", compat_mode);
        assert!(result.is_ok());

        let result = validate_provider_name("1234567890", compat_mode);
        assert!(result.is_ok());

        let result = validate_provider_name(
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890_",
            compat_mode,
        );
        assert!(result.is_ok());
    }
}
