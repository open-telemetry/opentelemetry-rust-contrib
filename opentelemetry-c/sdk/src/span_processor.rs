//! The generic span-processor handle (`otel_span_processor_t`) and its internal implementation
//! enum.
//!
//! The opaque C handle wraps a [`SpanProcessorImpl`] — an internal enum whose variants are the
//! concrete span-processor kinds this SDK supports. It implements
//! [`opentelemetry_sdk::trace::SpanProcessor`], so the SDK builder stores a homogeneous
//! `Vec<SpanProcessorImpl>` and drives every processor uniformly. The batch span processor is
//! one variant (SDK core, always available). Adding another processor kind (e.g. a simple span
//! processor) is a new variant plus a builder — no change to the public C ABI, the generic
//! handle, or the SDK builder's storage.

use std::time::Duration;

use opentelemetry::Context;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{BatchSpanProcessor, Span, SpanData, SpanProcessor};
use opentelemetry_sdk::Resource;

use crate::handle::{destroy, guard_unit, HasMagic};

pub(crate) const SPAN_PROCESSOR_MAGIC: u64 = 0x4F54_4C43_5350_5052; // "OTLCSPPR"

/// Internal span-processor implementation. Each variant is a concrete processor kind; the enum
/// dispatches the [`SpanProcessor`] trait to the active one. The batch processor is SDK core,
/// so this enum always has at least one variant.
#[derive(Debug)]
pub(crate) enum SpanProcessorImpl {
    /// Batch span processor (dedicated OS thread, spec-schedule export).
    Batch(BatchSpanProcessor),
}

impl SpanProcessor for SpanProcessorImpl {
    fn on_start(&self, span: &mut Span, cx: &Context) {
        match self {
            SpanProcessorImpl::Batch(p) => p.on_start(span, cx),
        }
    }
    fn on_end(&self, span: SpanData) {
        match self {
            SpanProcessorImpl::Batch(p) => p.on_end(span),
        }
    }
    fn force_flush(&self) -> OTelSdkResult {
        match self {
            SpanProcessorImpl::Batch(p) => p.force_flush(),
        }
    }
    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        match self {
            SpanProcessorImpl::Batch(p) => p.shutdown_with_timeout(timeout),
        }
    }
    fn set_resource(&mut self, resource: &Resource) {
        match self {
            SpanProcessorImpl::Batch(p) => p.set_resource(resource),
        }
    }
}

/// Opaque span-processor handle. Owns a built [`SpanProcessorImpl`] until it is consumed by the
/// SDK builder (via `add_span_processor`) or destroyed.
pub struct OtelSpanProcessor {
    magic: u64,
    pub(crate) processor: SpanProcessorImpl,
}

impl OtelSpanProcessor {
    pub(crate) fn new(processor: SpanProcessorImpl) -> Self {
        OtelSpanProcessor {
            magic: SPAN_PROCESSOR_MAGIC,
            processor,
        }
    }
}

impl HasMagic for OtelSpanProcessor {
    const MAGIC: u64 = SPAN_PROCESSOR_MAGIC;
    fn magic(&self) -> u64 {
        self.magic
    }
    fn set_magic(&mut self, value: u64) {
        self.magic = value;
    }
}

/// Destroy a span-processor handle (no-op on NULL).
///
/// Do **not** call this on a processor that was successfully transferred into an SDK builder
/// via `otel_sdk_builder_add_span_processor` — that builder owns it now (a transferred
/// handle's magic is poisoned, so this degrades to a safe no-op).
///
/// # Safety
/// `processor` must be NULL or a live processor handle, not destroyed concurrently.
#[no_mangle]
pub unsafe extern "C" fn otel_span_processor_destroy(processor: *mut OtelSpanProcessor) {
    guard_unit(|| unsafe { destroy(processor) });
}
