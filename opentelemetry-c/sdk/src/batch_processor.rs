//! The batch span processor builder (`otel_batch_span_processor_builder_t`).
//!
//! Consumes a [`OtelTraceExporter`] and batch settings, producing a generic
//! [`OtelSpanProcessor`] handle wrapping a `BatchSpanProcessor`. The processor owns a
//! dedicated OS thread and exports on the spec's batching schedule.
//!
//! Ownership: `set_exporter` transfers the exporter into this builder on `OTEL_STATUS_OK`
//! (the caller must not destroy it afterwards). Destroying this builder frees a transferred
//! exporter if `build` was not completed; a successful `build` moves it into the processor.

use std::time::Duration;

use opentelemetry_sdk::trace::{BatchConfigBuilder, BatchSpanProcessor};

use opentelemetry_c_abi::OtelStatus;

use crate::error::{clear_last_error, fail, fail_owned};
use crate::handle::{
    checked_mut, destroy, guard_ptr, guard_status, guard_unit, into_raw, take, HasMagic,
};
use crate::span_processor::OtelSpanProcessor;
use crate::trace_exporter::OtelTraceExporter;

const BATCH_PROCESSOR_BUILDER_MAGIC: u64 = 0x4F54_4C43_4253_5042; // "OTLCBSPB"

/// Upper bound on the batch max queue size accepted from C (the processor preallocates a
/// bounded channel of this capacity). `0` selects the SDK default; larger is rejected. This
/// protects against a C-driven allocation request before the channel is allocated.
const MAX_BATCH_QUEUE_SIZE: usize = 1_048_576;
/// Upper bound on the batch max export batch size accepted from C (a preallocated `Vec`).
const MAX_BATCH_EXPORT_BATCH_SIZE: usize = 1_048_576;

#[derive(Default)]
struct BatchConfig {
    max_queue_size: Option<usize>,
    scheduled_delay: Option<Duration>,
    max_export_batch_size: Option<usize>,
    export_timeout: Option<Duration>,
}

/// Opaque batch span processor builder. Not thread-safe; confine to one thread.
pub struct OtelBatchSpanProcessorBuilder {
    magic: u64,
    // Owned once `set_exporter` succeeds; freed on destroy or consumed by `build`.
    exporter: Option<Box<OtelTraceExporter>>,
    config: BatchConfig,
}

impl HasMagic for OtelBatchSpanProcessorBuilder {
    const MAGIC: u64 = BATCH_PROCESSOR_BUILDER_MAGIC;
    fn magic(&self) -> u64 {
        self.magic
    }
    fn set_magic(&mut self, value: u64) {
        self.magic = value;
    }
}

fn optional_millis(millis: u64) -> Option<Duration> {
    (millis != 0).then(|| Duration::from_millis(millis))
}

fn bounded_optional_size(
    value: usize,
    max: usize,
    what: &str,
) -> Result<Option<usize>, OtelStatus> {
    if value > max {
        return Err(fail_owned(
            OtelStatus::InvalidArgument,
            format!("{what} ({value}) exceeds the maximum supported value ({max})"),
        ));
    }
    Ok((value != 0).then_some(value))
}

/// Create a new batch span processor builder. Release with
/// `otel_batch_span_processor_builder_destroy()`.
#[no_mangle]
pub extern "C" fn otel_batch_span_processor_builder_new() -> *mut OtelBatchSpanProcessorBuilder {
    guard_ptr(|| {
        clear_last_error();
        into_raw(OtelBatchSpanProcessorBuilder {
            magic: BATCH_PROCESSOR_BUILDER_MAGIC,
            exporter: None,
            config: BatchConfig::default(),
        })
    })
}

/// Destroy a batch span processor builder (no-op on NULL). Frees a transferred exporter that
/// was not yet consumed by `build`.
///
/// # Safety
/// `builder` must be NULL or a live builder not destroyed concurrently.
#[no_mangle]
pub unsafe extern "C" fn otel_batch_span_processor_builder_destroy(
    builder: *mut OtelBatchSpanProcessorBuilder,
) {
    guard_unit(|| unsafe { destroy(builder) });
}

/// Set (transfer) the trace exporter this processor exports through. On `OTEL_STATUS_OK`,
/// ownership of `exporter` moves into the builder and the caller must not destroy it. On
/// failure (invalid builder or exporter), the caller still owns `exporter`. Replacing a
/// previously-set exporter frees the previous one.
///
/// # Safety
/// `builder` must satisfy the handle contract; `exporter` must be NULL or a live
/// `otel_trace_exporter_t` not used concurrently.
#[no_mangle]
pub unsafe extern "C" fn otel_batch_span_processor_builder_set_exporter(
    builder: *mut OtelBatchSpanProcessorBuilder,
    exporter: *mut OtelTraceExporter,
) -> OtelStatus {
    guard_status(|| {
        clear_last_error();
        // Validate the builder BEFORE taking ownership of the exporter, so a bad builder
        // leaves the exporter caller-owned.
        let builder = match unsafe { checked_mut(builder) } {
            Some(b) => b,
            None => return OtelStatus::InvalidArgument,
        };
        let owned = match unsafe { take::<OtelTraceExporter>(exporter) } {
            Some(e) => e,
            None => return OtelStatus::InvalidArgument,
        };
        builder.exporter = Some(owned);
        OtelStatus::Ok
    })
}

/// # Safety
/// `builder` must satisfy the handle contract (single-threaded).
unsafe fn with_config<F>(builder: *mut OtelBatchSpanProcessorBuilder, f: F) -> OtelStatus
where
    F: FnOnce(&mut BatchConfig) -> OtelStatus,
{
    guard_status(|| {
        clear_last_error();
        match unsafe { checked_mut(builder) } {
            Some(b) => f(&mut b.config),
            None => OtelStatus::InvalidArgument,
        }
    })
}

/// Set the maximum queue size (`0` == spec default of 2048). An oversized non-zero value is
/// rejected with `OTEL_STATUS_INVALID_ARGUMENT` (never clamped).
///
/// # Safety
/// `builder` must satisfy the handle contract.
#[no_mangle]
pub unsafe extern "C" fn otel_batch_span_processor_builder_set_max_queue_size(
    builder: *mut OtelBatchSpanProcessorBuilder,
    max_queue_size: usize,
) -> OtelStatus {
    unsafe {
        with_config(builder, |config| {
            match bounded_optional_size(
                max_queue_size,
                MAX_BATCH_QUEUE_SIZE,
                "batch max queue size",
            ) {
                Ok(size) => {
                    config.max_queue_size = size;
                    OtelStatus::Ok
                }
                Err(status) => status,
            }
        })
    }
}

/// Set the scheduled delay between exports in milliseconds (`0` == spec default).
///
/// # Safety
/// `builder` must satisfy the handle contract.
#[no_mangle]
pub unsafe extern "C" fn otel_batch_span_processor_builder_set_scheduled_delay_millis(
    builder: *mut OtelBatchSpanProcessorBuilder,
    delay_millis: u64,
) -> OtelStatus {
    unsafe {
        with_config(builder, |config| {
            config.scheduled_delay = optional_millis(delay_millis);
            OtelStatus::Ok
        })
    }
}

/// Set the maximum export batch size (`0` == spec default of 512). Oversized values are
/// rejected; the SDK also caps the effective value at the max queue size.
///
/// # Safety
/// `builder` must satisfy the handle contract.
#[no_mangle]
pub unsafe extern "C" fn otel_batch_span_processor_builder_set_max_export_batch_size(
    builder: *mut OtelBatchSpanProcessorBuilder,
    max_export_batch_size: usize,
) -> OtelStatus {
    unsafe {
        with_config(builder, |config| {
            match bounded_optional_size(
                max_export_batch_size,
                MAX_BATCH_EXPORT_BATCH_SIZE,
                "batch max export batch size",
            ) {
                Ok(size) => {
                    config.max_export_batch_size = size;
                    OtelStatus::Ok
                }
                Err(status) => status,
            }
        })
    }
}

/// Set the per-export timeout in milliseconds (`0` == spec default of 30000).
///
/// Note: the current stable synchronous batch span processor does not apply a programmatic
/// per-export timeout; it uses the SDK default (overridable via the `OTEL_BSP_EXPORT_TIMEOUT`
/// environment variable). This value is accepted and validated so the API shape is stable.
///
/// # Safety
/// `builder` must satisfy the handle contract.
#[no_mangle]
pub unsafe extern "C" fn otel_batch_span_processor_builder_set_export_timeout_millis(
    builder: *mut OtelBatchSpanProcessorBuilder,
    timeout_millis: u64,
) -> OtelStatus {
    unsafe {
        with_config(builder, |config| {
            config.export_timeout = optional_millis(timeout_millis);
            OtelStatus::Ok
        })
    }
}

fn build_processor(
    config: &BatchConfig,
    exporter: opentelemetry_otlp::SpanExporter,
) -> BatchSpanProcessor {
    let mut batch = BatchConfigBuilder::default();
    if let Some(size) = config.max_queue_size {
        batch = batch.with_max_queue_size(size);
    }
    if let Some(delay) = config.scheduled_delay {
        batch = batch.with_scheduled_delay(delay);
    }
    if let Some(size) = config.max_export_batch_size {
        batch = batch.with_max_export_batch_size(size);
    }
    // NOTE: the stable synchronous `BatchSpanProcessor` does not expose a programmatic
    // per-export timeout — `BatchConfigBuilder::with_max_export_timeout` is gated behind the
    // SDK's experimental async-runtime feature. The value is accepted and validated for a
    // stable API shape; on the stable processor the export timeout uses the SDK default
    // (30000 ms, overridable via the `OTEL_BSP_EXPORT_TIMEOUT` environment variable).
    let _ = config.export_timeout;
    BatchSpanProcessor::builder(exporter)
        .with_batch_config(batch.build())
        .build()
}

/// Build a span processor from the accumulated configuration. Requires an exporter set via
/// `set_exporter`. On `OTEL_STATUS_OK`, `*out` receives a new [`OtelSpanProcessor`] handle
/// owned by the caller (release with `otel_span_processor_destroy`, or transfer it into the
/// SDK builder). The exporter previously transferred here moves into the built processor. The
/// builder remains owned by the caller.
///
/// # Safety
/// `builder` must satisfy the handle contract; `out` a valid writable
/// `otel_span_processor_t*`.
#[no_mangle]
pub unsafe extern "C" fn otel_batch_span_processor_builder_build(
    builder: *mut OtelBatchSpanProcessorBuilder,
    out: *mut *mut OtelSpanProcessor,
) -> OtelStatus {
    guard_status(|| {
        clear_last_error();
        if out.is_null() {
            return fail(OtelStatus::InvalidArgument, "out pointer must not be NULL");
        }
        unsafe { *out = std::ptr::null_mut() };
        let builder = match unsafe { checked_mut(builder) } {
            Some(b) => b,
            None => return OtelStatus::InvalidArgument,
        };
        let exporter = match builder.exporter.take() {
            Some(e) => e.exporter,
            None => {
                return fail(
                    OtelStatus::InvalidConfig,
                    "batch span processor builder requires an exporter (call \
                     otel_batch_span_processor_builder_set_exporter first)",
                )
            }
        };
        let processor = build_processor(&builder.config, exporter);
        unsafe { *out = into_raw(OtelSpanProcessor::new(processor)) };
        OtelStatus::Ok
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::otlp_exporter::{
        otel_otlp_trace_exporter_builder_build, otel_otlp_trace_exporter_builder_destroy,
        otel_otlp_trace_exporter_builder_new, otel_otlp_trace_exporter_builder_set_endpoint,
    };
    use crate::span_processor::otel_span_processor_destroy;
    use crate::trace_exporter::otel_trace_exporter_destroy;
    use opentelemetry_c_abi::OtelStringView;

    fn sv(s: &str) -> OtelStringView {
        OtelStringView {
            ptr: s.as_ptr().cast::<std::os::raw::c_char>(),
            len: s.len(),
        }
    }

    /// Build a valid exporter handle (constructing it does not connect).
    fn make_exporter() -> *mut OtelTraceExporter {
        unsafe {
            let eb = otel_otlp_trace_exporter_builder_new();
            assert_eq!(
                otel_otlp_trace_exporter_builder_set_endpoint(
                    eb,
                    sv("http://127.0.0.1:9/v1/traces")
                ),
                OtelStatus::Ok
            );
            let mut exporter: *mut OtelTraceExporter = std::ptr::null_mut();
            assert_eq!(
                otel_otlp_trace_exporter_builder_build(eb, &mut exporter),
                OtelStatus::Ok
            );
            otel_otlp_trace_exporter_builder_destroy(eb);
            assert!(!exporter.is_null());
            exporter
        }
    }

    #[test]
    fn set_exporter_failure_leaves_exporter_caller_owned() {
        unsafe {
            let exporter = make_exporter();
            // A NULL (invalid) builder must NOT consume the exporter.
            assert_eq!(
                otel_batch_span_processor_builder_set_exporter(std::ptr::null_mut(), exporter),
                OtelStatus::InvalidArgument
            );
            // Still caller-owned: this frees it exactly once (no double-free).
            otel_trace_exporter_destroy(exporter);
        }
    }

    #[test]
    fn set_exporter_success_transfers_ownership() {
        unsafe {
            let exporter = make_exporter();
            let pb = otel_batch_span_processor_builder_new();
            assert_eq!(
                otel_batch_span_processor_builder_set_exporter(pb, exporter),
                OtelStatus::Ok
            );
            // Transferred: a destroy on the transferred handle is a poisoned no-op, and
            // destroying the builder frees the exporter exactly once.
            otel_trace_exporter_destroy(exporter);
            otel_batch_span_processor_builder_destroy(pb);
        }
    }

    #[test]
    fn build_without_exporter_is_invalid_config() {
        unsafe {
            let pb = otel_batch_span_processor_builder_new();
            let mut processor: *mut OtelSpanProcessor = std::ptr::null_mut();
            assert_eq!(
                otel_batch_span_processor_builder_build(pb, &mut processor),
                OtelStatus::InvalidConfig
            );
            assert!(processor.is_null());
            otel_batch_span_processor_builder_destroy(pb);
        }
    }

    #[test]
    fn build_success_consumes_exporter() {
        unsafe {
            let exporter = make_exporter();
            let pb = otel_batch_span_processor_builder_new();
            assert_eq!(
                otel_batch_span_processor_builder_set_exporter(pb, exporter),
                OtelStatus::Ok
            );
            let mut processor: *mut OtelSpanProcessor = std::ptr::null_mut();
            assert_eq!(
                otel_batch_span_processor_builder_build(pb, &mut processor),
                OtelStatus::Ok
            );
            assert!(!processor.is_null());
            // The exporter moved into the processor; a second build is InvalidConfig.
            let mut again: *mut OtelSpanProcessor = std::ptr::null_mut();
            assert_eq!(
                otel_batch_span_processor_builder_build(pb, &mut again),
                OtelStatus::InvalidConfig
            );
            otel_batch_span_processor_builder_destroy(pb);
            otel_span_processor_destroy(processor);
        }
    }

    #[test]
    fn batch_size_bounds_are_enforced() {
        unsafe {
            let pb = otel_batch_span_processor_builder_new();
            // 0 selects the SDK default; the maximum itself is accepted.
            assert_eq!(
                otel_batch_span_processor_builder_set_max_queue_size(pb, 0),
                OtelStatus::Ok
            );
            assert_eq!(
                otel_batch_span_processor_builder_set_max_queue_size(pb, MAX_BATCH_QUEUE_SIZE),
                OtelStatus::Ok
            );
            // One over the maximum is rejected (never clamped).
            assert_eq!(
                otel_batch_span_processor_builder_set_max_queue_size(pb, MAX_BATCH_QUEUE_SIZE + 1),
                OtelStatus::InvalidArgument
            );
            assert_eq!(
                otel_batch_span_processor_builder_set_max_export_batch_size(pb, usize::MAX),
                OtelStatus::InvalidArgument
            );
            assert_eq!(
                otel_batch_span_processor_builder_set_export_timeout_millis(pb, 1234),
                OtelStatus::Ok
            );
            otel_batch_span_processor_builder_destroy(pb);
        }
    }
}
