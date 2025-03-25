use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use opentelemetry::Context;
use opentelemetry_sdk::trace::Span;
use opentelemetry_sdk::trace::SpanExporter;
use opentelemetry_sdk::{error::OTelSdkResult, trace::SpanData};

/// This export processor exports without synchronization.
/// This is currently only used in user_events span exporter, where we know
/// that the underlying exporter is safe under concurrent calls.

#[derive(Debug)]
pub struct ReentrantSpanProcessor<T: SpanExporter> {
    // TODO - Mutex would be removed in future, once Exporter::export does not require mutable reference.
    exporter: Arc<Mutex<T>>,
}

impl<T: SpanExporter> ReentrantSpanProcessor<T> {
    /// Constructor that accepts an exporter instance.
    pub fn new(exporter: T) -> Self {
        ReentrantSpanProcessor {
            exporter: Arc::new(Mutex::new(exporter)),
        }
    }
}

impl<T: SpanExporter> opentelemetry_sdk::trace::SpanProcessor for ReentrantSpanProcessor<T> {
    fn on_start(&self, _span: &mut Span, _cx: &Context) { // No action needed on start.
    }

    fn on_end(&self, span: SpanData) {
        if let Ok(exporter) = self.exporter.lock() {
            let _ = futures_executor::block_on(exporter.export(vec![span]));
        }
    }

    // Ensures all spans are flushed.
    fn force_flush(&self) -> OTelSdkResult {
        if let Ok(mut exporter) = self.exporter.lock() {
            exporter.force_flush()
        } else {
            Ok(())
        }
    }

    // Properly shuts down the exporter.
    fn shutdown(&self) -> OTelSdkResult {
        if let Ok(mut exporter) = self.exporter.lock() {
            exporter.shutdown()
        } else {
            Ok(())
        }
    }
}
