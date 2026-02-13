//! SpanProcessor implementation that exports spans to ETW.
//!
//! Provides `Processor` and `ProcessorBuilder` for configuring and creating
//! the ETW span processor.

use crate::exporter::{options::Options, ETWExporter};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{Span, SpanData, SpanProcessor};
use std::borrow::Cow;
use std::error::Error;
use std::fmt::Debug;
use std::time::Duration;

/// Processes and exports spans to ETW.
///
/// This processor exports spans without synchronization.
/// It is specifically designed for the ETW exporter, where
/// the underlying exporter is safe under concurrent calls.
///
/// Implements [`SpanProcessor`], so it can be used with
/// [`SdkTracerProvider`](opentelemetry_sdk::trace::SdkTracerProvider).
#[derive(Debug)]
pub struct Processor {
    event_exporter: ETWExporter,
}

impl Processor {
    /// Creates a new instance of [`ProcessorBuilder`] with the given provider name.
    ///
    /// The provider name must be cross-compatible with UserEvents (Linux) and must conform
    /// to the following requirements:
    ///
    /// - non-empty,
    /// - less than 234 characters long, and
    /// - contain only ASCII alphanumeric characters or underscores (`_`).
    ///
    /// By default, all events will be exported to the "Span" ETW event.
    /// See [`ProcessorBuilder`] for details on how to configure a different behavior.
    pub fn builder(provider_name: &str) -> ProcessorBuilder {
        ProcessorBuilder::new(provider_name)
    }

    /// Creates a new instance of [`ProcessorBuilder`] with the given provider name
    /// conforming to ETW requirements only.
    ///
    /// Cross-compatibility with UserEvents (Linux) is not guaranteed. Requirements:
    ///
    /// - non-empty,
    /// - less than 234 characters long, and
    /// - contain only ASCII alphanumeric characters, underscores (`_`) or hyphens (`-`).
    pub fn builder_etw_compat_only(provider_name: &str) -> ProcessorBuilder {
        ProcessorBuilder::new_etw_compat_only(provider_name)
    }

    /// Creates a new instance of the [`Processor`] using the given options.
    pub(crate) fn new(options: Options) -> Self {
        let exporter = ETWExporter::new(options);
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

impl SpanProcessor for Processor {
    fn on_start(&self, _span: &mut Span, _cx: &opentelemetry::Context) {
        // No action needed on span start for ETW export.
    }

    fn on_end(&self, span: SpanData) {
        self.event_exporter.export_span_data(&span);
    }

    // Nothing to flush as this processor does not buffer
    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        self.event_exporter.shutdown()
    }

    fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
        self.event_exporter.set_resource(resource);
    }
}

/// Builder for configuring and constructing a [`Processor`].
///
/// # Example
///
/// ```no_run
/// use opentelemetry_etw_traces::Processor;
///
/// let processor = Processor::builder("MyProviderName")
///     .with_event_name("Span")
///     .build()
///     .expect("Failed to create processor");
/// ```
#[derive(Debug)]
pub struct ProcessorBuilder {
    options: Options,
    provider_name_compat_mode: ProviderNameCompatMode,
}

impl ProcessorBuilder {
    /// Creates a new instance of [`ProcessorBuilder`] with the given provider name.
    ///
    /// The provider name must contain only ASCII alphanumeric characters or '_'.
    pub(crate) fn new(provider_name: &str) -> Self {
        ProcessorBuilder {
            options: Options::new(provider_name.to_string()),
            provider_name_compat_mode: ProviderNameCompatMode::CrossCompat,
        }
    }

    /// Creates a new instance of [`ProcessorBuilder`] with the given provider name.
    ///
    /// The provider name must contain only ASCII alphanumeric characters, '_' or '-'.
    pub(crate) fn new_etw_compat_only(provider_name: &str) -> Self {
        ProcessorBuilder {
            options: Options::new(provider_name.to_string()),
            provider_name_compat_mode: ProviderNameCompatMode::EtwCompatOnly,
        }
    }

    /// Sets the default ETW event name (default: "Span").
    pub fn with_event_name(mut self, name: &str) -> Self {
        self.options = self.options.with_event_name(name);
        self
    }

    /// Specifies additional resource attribute keys to include in Part C.
    ///
    /// By default, only `service.name` and `service.instance.id` are extracted
    /// (as `cloud.role` and `cloud.roleInstance` in Part A).
    /// Use this to promote additional resource attributes into the ETW event.
    ///
    /// # Performance Considerations
    ///
    /// **Warning**: Each specified resource attribute will be serialized and sent
    /// with EVERY span. Consider the performance impact when selecting which
    /// attributes to export.
    pub fn with_resource_attributes<I, S>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Cow<'static, str>>,
    {
        self.options = self.options.with_resource_attributes(keys);
        self
    }

    /// Builds the `Processor`, registering the ETW provider.
    ///
    /// Returns an error if the provider name is invalid.
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
    use opentelemetry_sdk::trace::SpanProcessor;

    #[test]
    fn test_shutdown() {
        let processor = Processor::new(test_options());
        assert!(processor
            .shutdown_with_timeout(Duration::from_secs(5))
            .is_ok());
    }

    #[test]
    fn test_force_flush() {
        let processor = Processor::new(test_options());
        assert!(processor.force_flush().is_ok());
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
        let compat_mode = ProviderNameCompatMode::CrossCompat;

        assert!(validate_provider_name("valid_provider_name", compat_mode).is_ok());
        assert!(validate_provider_name("", compat_mode).is_err());
        assert!(validate_provider_name("a".repeat(235).as_str(), compat_mode).is_err());
        assert!(validate_provider_name("i_have_a_-_", compat_mode).is_err());
        assert!(validate_provider_name("_?_", compat_mode).is_err());
        assert!(validate_provider_name("abcdefghijklmnopqrstuvwxyz", compat_mode).is_ok());
        assert!(validate_provider_name("ABCDEFGHIJKLMNOPQRSTUVWXYZ", compat_mode).is_ok());
        assert!(validate_provider_name("1234567890", compat_mode).is_ok());
        assert!(validate_provider_name(
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890_",
            compat_mode,
        )
        .is_ok());
    }

    #[test]
    fn test_validate_provider_name_etw_compat() {
        let compat_mode = ProviderNameCompatMode::EtwCompatOnly;

        assert!(validate_provider_name("valid_provider_name", compat_mode).is_ok());
        assert!(validate_provider_name("", compat_mode).is_err());
        assert!(validate_provider_name("a".repeat(235).as_str(), compat_mode).is_err());
        assert!(validate_provider_name("i_have_a_-_", compat_mode).is_ok());
        assert!(validate_provider_name("_?_", compat_mode).is_err());
        assert!(validate_provider_name("abcdefghijklmnopqrstuvwxyz", compat_mode).is_ok());
        assert!(validate_provider_name("ABCDEFGHIJKLMNOPQRSTUVWXYZ", compat_mode).is_ok());
        assert!(validate_provider_name("1234567890", compat_mode).is_ok());
        assert!(validate_provider_name(
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890_",
            compat_mode,
        )
        .is_ok());
    }

    #[test]
    fn test_with_resource_attributes() {
        use opentelemetry::KeyValue;
        use opentelemetry_sdk::Resource;

        let processor = Processor::builder("test_provider")
            .with_resource_attributes(vec!["custom_attribute1", "custom_attribute2"])
            .build()
            .unwrap();

        let mut processor = processor;

        let resource = Resource::builder()
            .with_attributes([
                KeyValue::new("service.name", "test-service"),
                KeyValue::new("service.instance.id", "test-instance"),
                KeyValue::new("custom_attribute1", "value1"),
                KeyValue::new("custom_attribute2", "value2"),
                KeyValue::new("custom_attribute3", "value3"), // This should be ignored
            ])
            .build();

        processor.set_resource(&resource);
        assert!(processor.force_flush().is_ok());
    }

    fn test_options() -> Options {
        Options::new("test_provider_name")
    }
}
