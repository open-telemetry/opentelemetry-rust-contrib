//! The generic trace-exporter handle (`otel_trace_exporter_t`).
//!
//! This is the opaque object a span-processor builder consumes. Today it always wraps the
//! OTLP HTTP/protobuf exporter (built via [`crate::otlp_exporter`]); the opaque C handle lets
//! other exporter kinds be added later — the inner field would become an enum — without
//! changing the public C ABI.

use opentelemetry_otlp::SpanExporter;

use crate::handle::{destroy, guard_unit, HasMagic};

pub(crate) const TRACE_EXPORTER_MAGIC: u64 = 0x4F54_4C43_5452_4558; // "OTLCTREX"

/// Opaque trace-exporter handle. Owns a built span exporter until it is consumed by a span
/// processor builder (via `set_exporter`) or destroyed.
pub struct OtelTraceExporter {
    magic: u64,
    // Currently always the OTLP HTTP/protobuf exporter. See the module docs.
    pub(crate) exporter: SpanExporter,
}

impl OtelTraceExporter {
    pub(crate) fn new(exporter: SpanExporter) -> Self {
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
