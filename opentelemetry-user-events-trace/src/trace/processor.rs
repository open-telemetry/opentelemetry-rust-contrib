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
}

impl<'a> ProcessorBuilder<'a> {
    fn new(provider_name: &'a str) -> Self {
        Self { provider_name }
    }

    /// Builds the processor, returning an error if its configuration is invalid.
    pub fn build(self) -> Result<Processor, Box<dyn Error>> {
        if self.provider_name.is_empty() {
            return Err("Provider name cannot be empty.".into());
        }

        let exporter = UserEventsSpanExporter::new(self.provider_name)?;
        Ok(Processor { exporter })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct FilteringProcessor {
        inner: Processor,
        export_spans: bool,
    }

    impl SpanProcessor for FilteringProcessor {
        fn on_start(&self, span: &mut Span, cx: &Context) {
            self.inner.on_start(span, cx);
        }

        fn on_end(&self, span: SpanData) {
            if self.export_spans {
                self.inner.on_end(span);
            }
        }

        fn force_flush(&self) -> OTelSdkResult {
            self.inner.force_flush()
        }

        fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
            self.inner.shutdown_with_timeout(timeout)
        }

        fn set_resource(&mut self, resource: &Resource) {
            self.inner.set_resource(resource);
        }
    }

    #[test]
    fn processor_can_be_wrapped() {
        let inner = Processor::builder("test_provider").build().unwrap();
        let processor = FilteringProcessor {
            inner,
            export_spans: false,
        };

        let _provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_span_processor(processor)
            .build();
    }

    #[test]
    fn empty_provider_name_is_rejected() {
        assert_eq!(
            Processor::builder("").build().unwrap_err().to_string(),
            "Provider name cannot be empty."
        );
    }
}
