//! The generic span-processor handle (`otel_span_processor_t`).
//!
//! This is the opaque object the SDK builder consumes via `add_span_processor`. Today it
//! always wraps a batch span processor (built via [`crate::batch_processor`]); the opaque C
//! handle lets other processor kinds be added later — the inner field would become an enum —
//! without changing the public C ABI.

use opentelemetry_sdk::trace::BatchSpanProcessor;

use crate::handle::{destroy, guard_unit, HasMagic};

pub(crate) const SPAN_PROCESSOR_MAGIC: u64 = 0x4F54_4C43_5350_5052; // "OTLCSPPR"

/// Opaque span-processor handle. Owns a built processor until it is consumed by the SDK
/// builder (via `add_span_processor`) or destroyed.
pub struct OtelSpanProcessor {
    magic: u64,
    // Currently always a batch span processor. See the module docs.
    pub(crate) processor: BatchSpanProcessor,
}

impl OtelSpanProcessor {
    pub(crate) fn new(processor: BatchSpanProcessor) -> Self {
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
