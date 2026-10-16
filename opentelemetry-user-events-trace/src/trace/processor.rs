use std::borrow::Cow;
use std::collections::HashSet;
use std::error::Error;
use std::fmt::Debug;
use std::time::Duration;

use opentelemetry::Context;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{Span, SpanData, SpanExporter, SpanProcessor};
use opentelemetry_sdk::Resource;

use super::exporter::UserEventsSpanExporter;

/// Processes and exports spans to user_events.
///
/// This processor exports spans synchronously without buffering.
/// It can be wrapped by another [`SpanProcessor`], for example to filter spans
/// before they are exported.
///
/// ```no_run
/// use opentelemetry::Context;
/// use opentelemetry_sdk::trace::{SdkTracerProvider, Span, SpanData, SpanProcessor};
/// use opentelemetry_user_events_trace::Processor;
///
/// #[derive(Debug)]
/// struct FilteringProcessor<P> {
///     inner: P,
/// }
///
/// impl<P: SpanProcessor> SpanProcessor for FilteringProcessor<P> {
///     fn on_start(&self, span: &mut Span, cx: &Context) {
///         self.inner.on_start(span, cx);
///     }
///
///     fn on_end(&self, span: SpanData) {
///         if span.name != "health-check" {
///             self.inner.on_end(span);
///         }
///     }
///
///     fn force_flush(&self) -> opentelemetry_sdk::error::OTelSdkResult {
///         self.inner.force_flush()
///     }
///
///     fn shutdown_with_timeout(
///         &self,
///         timeout: std::time::Duration,
///     ) -> opentelemetry_sdk::error::OTelSdkResult {
///         self.inner.shutdown_with_timeout(timeout)
///     }
///
///     fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
///         self.inner.set_resource(resource);
///     }
/// }
///
/// let processor = Processor::builder("my_provider").build()?;
/// let provider = SdkTracerProvider::builder()
///     .with_span_processor(FilteringProcessor { inner: processor })
///     .build();
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct Processor {
    exporter: UserEventsSpanExporter,
}

impl Processor {
    /// Creates a builder for configuring a user_events processor.
    ///
    /// The provider name must:
    ///
    /// - not be empty,
    /// - be less than 234 characters, and
    /// - contain only ASCII letters, digits, and underscores (`_`).
    ///
    /// A provider named `my_provider` creates the tracepoint
    /// `user_events:my_provider_L4K1`. The exporter reads `service.name` and
    /// `service.instance.id` from the provider resource and writes them as
    /// `ext_cloud_role` and `ext_cloud_roleInstance`, respectively.
    pub fn builder(provider_name: &str) -> ProcessorBuilder<'_> {
        ProcessorBuilder::new(provider_name)
    }
}

impl SpanProcessor for Processor {
    fn on_start(&self, _span: &mut Span, _cx: &Context) {}

    fn on_end(&self, span: SpanData) {
        let _ = self.exporter.export_span(&span);
    }

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown(&self) -> OTelSdkResult {
        self.exporter.shutdown()
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        self.shutdown()
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.exporter.set_resource(resource);
    }
}

/// Builder for configuring and constructing a user_events [`Processor`].
#[derive(Debug)]
pub struct ProcessorBuilder<'a> {
    provider_name: &'a str,
    resource_attribute_keys: HashSet<Cow<'static, str>>,
}

impl<'a> ProcessorBuilder<'a> {
    fn new(provider_name: &'a str) -> Self {
        Self {
            provider_name,
            resource_attribute_keys: HashSet::new(),
        }
    }

    /// Specifies additional resource attribute keys to export in Part C.
    ///
    /// By default, only `service.name` and `service.instance.id` are exported,
    /// as Part A fields. They remain in Part A even when selected here.
    /// Other selected attributes are appended after span attributes, retaining
    /// duplicate keys. Missing attributes are ignored. Calling this method again
    /// replaces the previous selection.
    ///
    /// # Performance Considerations
    ///
    /// Each selected resource attribute is serialized and sent with EVERY span,
    /// rather than once per batch as in OTLP. Be selective: a local agent can
    /// often determine host, infrastructure, and deployment attributes itself.
    /// Prefer attributes specific to your application instance.
    ///
    /// Values are cached when the provider sets the resource, avoiding the need
    /// to attach these attributes explicitly to every span.
    pub fn with_resource_attributes<I, S>(mut self, attributes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Cow<'static, str>>,
    {
        self.resource_attribute_keys = attributes.into_iter().map(Into::into).collect();
        self
    }

    /// Builds the processor, returning an error if its configuration is invalid.
    pub fn build(self) -> Result<Processor, Box<dyn Error>> {
        if self.provider_name.is_empty() {
            return Err("Provider name cannot be empty.".into());
        }

        let exporter =
            UserEventsSpanExporter::new(self.provider_name, self.resource_attribute_keys)?;
        Ok(Processor { exporter })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_attribute_selection() {
        let builder = Processor::builder("test_provider");
        assert!(builder.resource_attribute_keys.is_empty());

        let builder = builder.with_resource_attributes(["service.version", "service.version"]);
        assert_eq!(builder.resource_attribute_keys.len(), 1);
        assert!(builder.resource_attribute_keys.contains("service.version"));

        let builder = builder.with_resource_attributes(vec!["custom.attribute".to_string()]);
        assert_eq!(builder.resource_attribute_keys.len(), 1);
        assert!(builder.resource_attribute_keys.contains("custom.attribute"));
        assert!(!builder.resource_attribute_keys.contains("service.version"));
        assert!(builder.build().is_ok());
    }

    #[test]
    fn processor_lifecycle() {
        let mut processor = Processor::builder("test_provider").build().unwrap();
        processor.set_resource(
            &Resource::builder()
                .with_service_name("test-service")
                .build(),
        );

        assert!(processor.force_flush().is_ok());
        assert!(processor
            .shutdown_with_timeout(Duration::from_secs(1))
            .is_ok());
    }

    #[test]
    fn empty_provider_name_is_rejected() {
        assert_eq!(
            Processor::builder("").build().unwrap_err().to_string(),
            "Provider name cannot be empty."
        );
    }

    #[test]
    fn invalid_provider_name_is_rejected() {
        assert_eq!(
            Processor::builder("invalid-provider")
                .build()
                .unwrap_err()
                .to_string(),
            "Provider name must contain only ASCII letters, digits, and '_'."
        );
    }
}
