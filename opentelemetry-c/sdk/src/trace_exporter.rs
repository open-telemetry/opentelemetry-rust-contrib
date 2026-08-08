//! The generic trace-exporter handle (`otel_trace_exporter_t`) and its internal
//! implementation enum.
//!
//! The opaque C handle wraps a `TraceExporterImpl` — an internal enum whose variants are the
//! concrete exporter kinds this SDK supports. It implements
//! [`opentelemetry_sdk::trace::SpanExporter`], so a span processor can drive it uniformly
//! regardless of which exporter is inside. The OTLP HTTP/protobuf exporter is one **optional**
//! variant (feature `otlp`), not SDK core: with `--no-default-features` the enum has no
//! variants and the SDK core still builds. Adding a new exporter kind is a new variant plus a
//! builder — no change to the public C ABI or handle shape.

use std::time::Duration;

use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use opentelemetry_sdk::Resource;

use crate::handle::{destroy, guard_unit, HasMagic};

pub(crate) const TRACE_EXPORTER_MAGIC: u64 = 0x4F54_4C43_5452_4558; // "OTLCTREX"

/// Internal trace-exporter implementation. Each variant is a concrete exporter kind; the enum
/// dispatches the [`SpanExporter`] trait to the active one. OTLP is optional (`otlp` feature);
/// with no exporter feature enabled the enum is uninhabited and cannot be constructed.
#[derive(Debug)]
pub(crate) enum TraceExporterImpl {
    /// OTLP HTTP/protobuf exporter (optional; feature `otlp`).
    #[cfg(feature = "otlp")]
    Otlp(opentelemetry_otlp::SpanExporter),
}

// Dispatch the SpanExporter trait to the active variant. Split by feature so the OTLP-disabled
// build (an uninhabited enum) is handled without an unreachable placeholder variant.
#[cfg(feature = "otlp")]
impl SpanExporter for TraceExporterImpl {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        match self {
            TraceExporterImpl::Otlp(inner) => inner.export(batch).await,
        }
    }
    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        match self {
            TraceExporterImpl::Otlp(inner) => inner.shutdown_with_timeout(timeout),
        }
    }
    fn force_flush(&self) -> OTelSdkResult {
        match self {
            TraceExporterImpl::Otlp(inner) => inner.force_flush(),
        }
    }
    fn set_resource(&mut self, resource: &Resource) {
        match self {
            TraceExporterImpl::Otlp(inner) => inner.set_resource(resource),
        }
    }
}

#[cfg(not(feature = "otlp"))]
impl SpanExporter for TraceExporterImpl {
    async fn export(&self, _batch: Vec<SpanData>) -> OTelSdkResult {
        // Uninhabited when no exporter feature is enabled: cannot be constructed or called.
        match *self {}
    }
    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        match *self {}
    }
    fn force_flush(&self) -> OTelSdkResult {
        match *self {}
    }
    fn set_resource(&mut self, _resource: &Resource) {
        match *self {}
    }
}

/// Opaque trace-exporter handle. Owns a built `TraceExporterImpl` until it is consumed by a
/// span processor builder (via `set_exporter`) or destroyed.
pub struct OtelTraceExporter {
    magic: u64,
    pub(crate) exporter: TraceExporterImpl,
}

impl OtelTraceExporter {
    pub(crate) fn new(exporter: TraceExporterImpl) -> Self {
        OtelTraceExporter {
            magic: TRACE_EXPORTER_MAGIC,
            exporter,
        }
    }
}

impl HasMagic for OtelTraceExporter {
    const MAGIC: u64 = TRACE_EXPORTER_MAGIC;
    fn magic(&self) -> u64 {
        self.magic
    }
    fn set_magic(&mut self, value: u64) {
        self.magic = value;
    }
}

/// Destroy a trace-exporter handle (no-op on NULL).
///
/// Do **not** call this on an exporter that was successfully transferred into a span
/// processor builder via `otel_batch_span_processor_builder_set_exporter` — that builder owns
/// it now (a transferred handle's magic is poisoned, so this degrades to a safe no-op).
///
/// # Safety
/// `exporter` must be NULL or a live exporter handle, not destroyed concurrently.
#[no_mangle]
pub unsafe extern "C" fn otel_trace_exporter_destroy(exporter: *mut OtelTraceExporter) {
    guard_unit(|| unsafe { destroy(exporter) });
}
