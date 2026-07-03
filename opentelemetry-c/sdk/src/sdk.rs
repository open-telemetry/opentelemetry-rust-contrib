//! SDK builder and lifecycle.
//!
//! The [`OtelSdkBuilder`] holds resource/service configuration and a list of span processors,
//! then builds an [`OtelSdk`] that owns the resulting `SdkTracerProvider`. Trace exporters and
//! span processors are configured by their own builders ([`crate::otlp_exporter`],
//! [`crate::batch_processor`]) and handed to this builder via `add_span_processor`, so the SDK
//! builder is not coupled to any one exporter or processor implementation.
//!
//! Installing as global (or fetching a provider handle) registers the SDK's implementation
//! into the **API cdylib's** global slot across the C ABI.

use std::os::raw::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry_sdk::trace::{BatchSpanProcessor, SdkTracerProvider};
use opentelemetry_sdk::Resource;

use opentelemetry_c_abi::{OtelKeyValue, OtelStatus, OtelStringView};

use crate::api_ffi;
use crate::error::{clear_last_error, fail, fail_owned, status_from_sdk_error};
use crate::handle::{
    checked_mut, checked_ref, destroy, guard_ptr, guard_status, guard_unit, into_raw, take,
    HasMagic,
};
use crate::span_processor::OtelSpanProcessor;
use crate::vtable;

const SDK_BUILDER_MAGIC: u64 = 0x4F54_4C43_5344_4B42; // "OTLCSDKB"
const SDK_MAGIC: u64 = 0x4F54_4C43_5344_4B00; // "OTLCSDK\0"

/// Opaque builder handle (`otel_sdk_builder_t`). Not thread-safe; confine to one thread.
pub struct OtelSdkBuilder {
    magic: u64,
    service_name: Option<String>,
    resource_attributes: Vec<KeyValue>,
    // Span processors transferred in via `add_span_processor`; moved into the provider on
    // `build`, or freed on destroy if `build` was not completed. Currently the concrete
    // `BatchSpanProcessor`; adding other processor kinds would generalize this to an enum.
    processors: Vec<BatchSpanProcessor>,
}

impl HasMagic for OtelSdkBuilder {
    const MAGIC: u64 = SDK_BUILDER_MAGIC;
    fn magic(&self) -> u64 {
        self.magic
    }
    fn set_magic(&mut self, value: u64) {
        self.magic = value;
    }
}

/// Opaque SDK handle (`otel_sdk_t`). All operations except destroy take shared access.
pub struct OtelSdk {
    magic: u64,
    provider: SdkTracerProvider,
    shutdown: AtomicBool,
    flush_in_flight: Arc<AtomicBool>,
}

impl HasMagic for OtelSdk {
    const MAGIC: u64 = SDK_MAGIC;
    fn magic(&self) -> u64 {
        self.magic
    }
    fn set_magic(&mut self, value: u64) {
        self.magic = value;
    }
}

// Concurrent C callers share one SDK by raw pointer; every non-destroy op forms `&OtelSdk`,
// sound across threads only if `OtelSdk: Sync`. Asserted here.
const _: () = {
    fn assert_sync<T: Sync>() {}
    let _ = assert_sync::<OtelSdk>;
};

fn optional_millis(millis: u64) -> Option<Duration> {
    (millis != 0).then(|| Duration::from_millis(millis))
}

// ---- Builder lifecycle -----------------------------------------------------

/// Create a new SDK builder with spec-default settings. Release with `otel_sdk_builder_destroy()`.
#[no_mangle]
pub extern "C" fn otel_sdk_builder_new() -> *mut OtelSdkBuilder {
    guard_ptr(|| {
        clear_last_error();
        into_raw(OtelSdkBuilder {
            magic: SDK_BUILDER_MAGIC,
            service_name: None,
            resource_attributes: Vec::new(),
            processors: Vec::new(),
        })
    })
}

/// Destroy an SDK builder (no-op on NULL). Frees any span processors transferred in but not
/// yet consumed by `otel_sdk_build`.
///
/// # Safety
/// `builder` must be NULL or a live builder not destroyed concurrently.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_builder_destroy(builder: *mut OtelSdkBuilder) {
    guard_unit(|| unsafe { destroy(builder) });
}

/// # Safety
/// `builder` must satisfy the handle contract (single-threaded).
unsafe fn with_builder<F>(builder: *mut OtelSdkBuilder, f: F) -> OtelStatus
where
    F: FnOnce(&mut OtelSdkBuilder) -> OtelStatus,
{
    guard_status(|| {
        clear_last_error();
        match unsafe { checked_mut(builder) } {
            Some(b) => f(b),
            None => OtelStatus::InvalidArgument,
        }
    })
}

/// Set the `service.name` resource attribute.
///
/// # Safety
/// `builder` and `name` must satisfy their contracts.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_builder_set_service_name(
    builder: *mut OtelSdkBuilder,
    name: OtelStringView,
) -> OtelStatus {
    unsafe {
        with_builder(builder, |b| match name.to_string_strict() {
            Ok(name) => {
                b.service_name = Some(name);
                OtelStatus::Ok
            }
            Err(e) => crate::error::fail_abi(e),
        })
    }
}

/// Add an arbitrary resource attribute.
///
/// # Safety
/// `builder` and `attribute` must satisfy their contracts.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_builder_add_resource_attribute(
    builder: *mut OtelSdkBuilder,
    attribute: OtelKeyValue,
) -> OtelStatus {
    unsafe {
        with_builder(builder, |b| match vtable_to_key_value(&attribute) {
            Ok(kv) => {
                b.resource_attributes.push(kv);
                OtelStatus::Ok
            }
            Err(status) => status,
        })
    }
}

/// Add (transfer) a span processor built by a span-processor builder. On `OTEL_STATUS_OK`,
/// ownership of `processor` moves into the SDK builder and the caller must not destroy it. On
/// failure (invalid builder or processor), the caller still owns `processor`.
///
/// # Safety
/// `builder` must satisfy the handle contract; `processor` must be NULL or a live
/// `otel_span_processor_t` not used concurrently.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_builder_add_span_processor(
    builder: *mut OtelSdkBuilder,
    processor: *mut OtelSpanProcessor,
) -> OtelStatus {
    guard_status(|| {
        clear_last_error();
        // Validate the builder BEFORE taking ownership, so a bad builder leaves the processor
        // caller-owned.
        let builder = match unsafe { checked_mut(builder) } {
            Some(b) => b,
            None => return OtelStatus::InvalidArgument,
        };
        let owned = match unsafe { take::<OtelSpanProcessor>(processor) } {
            Some(p) => p,
            None => return OtelStatus::InvalidArgument,
        };
        builder.processors.push(owned.processor);
        OtelStatus::Ok
    })
}

// ---- Build -----------------------------------------------------------------

/// Convert a C attribute into an owned `KeyValue` (used for resource attributes).
fn vtable_to_key_value(kv: &OtelKeyValue) -> Result<KeyValue, OtelStatus> {
    // SAFETY: the builder attribute satisfies the OtelKeyValue string contract.
    unsafe { crate::vtable::to_key_value(kv) }
}

fn build_resource(builder: &OtelSdkBuilder) -> Resource {
    let mut resource = Resource::builder();
    if let Some(name) = &builder.service_name {
        resource = resource.with_service_name(name.clone());
    }
    if !builder.resource_attributes.is_empty() {
        resource = resource.with_attributes(builder.resource_attributes.iter().cloned());
    }
    resource.build()
}

/// Build an SDK from the accumulated builder configuration. On `OTEL_STATUS_OK`, `*out_sdk`
/// receives a new [`OtelSdk`] handle owned by the caller. Any span processors added to the
/// builder move into the built SDK. The builder remains owned by the caller (destroy it when
/// done); note that a subsequent build produces an SDK with no processors.
///
/// # Safety
/// `builder` must satisfy the handle contract; `out_sdk` a valid writable `otel_sdk_t*`.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_build(
    builder: *mut OtelSdkBuilder,
    out_sdk: *mut *mut OtelSdk,
) -> OtelStatus {
    guard_status(|| {
        clear_last_error();
        if out_sdk.is_null() {
            return fail(
                OtelStatus::InvalidArgument,
                "out_sdk pointer must not be NULL",
            );
        }
        unsafe { *out_sdk = std::ptr::null_mut() };
        let builder = match unsafe { checked_mut(builder) } {
            Some(b) => b,
            None => return OtelStatus::InvalidArgument,
        };
        // Move the transferred processors out of the builder into the provider.
        let processors = std::mem::take(&mut builder.processors);
        let resource = build_resource(builder);
        let mut provider_builder = SdkTracerProvider::builder().with_resource(resource);
        for processor in processors {
            provider_builder = provider_builder.with_span_processor(processor);
        }
        let provider = provider_builder.build();
        let sdk = into_raw(OtelSdk {
            magic: SDK_MAGIC,
            provider,
            shutdown: AtomicBool::new(false),
            flush_in_flight: Arc::new(AtomicBool::new(false)),
        });
        unsafe { *out_sdk = sdk };
        OtelStatus::Ok
    })
}

// ---- Provider access and global installation -------------------------------

/// Return an owned tracer-provider handle backed by this SDK. The returned pointer is an
/// API `otel_tracer_provider_t*` (allocated by the API cdylib); release it with
/// `otel_tracer_provider_destroy()`. Returns NULL if `sdk` is invalid.
///
/// # Safety
/// `sdk` must satisfy the handle contract.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_get_tracer_provider(sdk: *const OtelSdk) -> *mut c_void {
    guard_ptr(|| {
        clear_last_error();
        match unsafe { checked_ref::<OtelSdk>(sdk) } {
            Some(sdk) => {
                let ctx = vtable::provider_ctx(sdk.provider.clone());
                let handle = api_ffi::provider_new(vtable::vtable_ptr(), ctx);
                if handle.is_null() {
                    // The API rejected it; free the context we allocated.
                    (vtable::SDK_VTABLE.provider_free)(ctx);
                }
                handle
            }
            None => std::ptr::null_mut(),
        }
    })
}

/// Install this SDK's tracer provider as the process-global provider (in the API-owned
/// slot). May be called more than once; the most recent call wins. Returns
/// `OTEL_STATUS_ALREADY_SHUTDOWN` if the SDK has been shut down.
///
/// # Library lifetime
/// On success this publishes the crate's `'static` vtable and an SDK-owned provider object
/// into the API global slot. Neither [`otel_sdk_shutdown`] nor [`otel_sdk_destroy`] clears
/// that slot; it is cleared only when another provider replaces it. So after a successful
/// install, `libopentelemetry_c_sdk` must remain loaded until process exit or until another
/// provider replaces the slot — shutdown + destroy do **not** make unloading the SDK safe.
///
/// # Safety
/// `sdk` must satisfy the handle contract and must not be destroyed concurrently.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_set_as_global(sdk: *mut OtelSdk) -> OtelStatus {
    guard_status(|| {
        clear_last_error();
        let sdk = match unsafe { checked_ref::<OtelSdk>(sdk) } {
            Some(s) => s,
            None => return OtelStatus::InvalidArgument,
        };
        if sdk.shutdown.load(Ordering::Acquire) {
            return fail(
                OtelStatus::AlreadyShutdown,
                "cannot install a shut-down SDK as global",
            );
        }
        let ctx = vtable::provider_ctx(sdk.provider.clone());
        let status = api_ffi::register_global_provider(vtable::vtable_ptr(), ctx);
        if status != OtelStatus::Ok {
            (vtable::SDK_VTABLE.provider_free)(ctx);
        }
        status
    })
}

// ---- Lifecycle -------------------------------------------------------------

fn map_flush_result(result: opentelemetry_sdk::error::OTelSdkResult) -> OtelStatus {
    match result {
        Ok(()) => OtelStatus::Ok,
        Err(err) => status_from_sdk_error(&err),
    }
}

/// Clears the shared force-flush in-flight flag on drop (even if the flush panics).
struct FlushGuard(Arc<AtomicBool>);
impl Drop for FlushGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Flush any buffered spans. `timeout_millis == 0` blocks on the calling thread; otherwise
/// the flush runs on a helper thread (at most one in flight) and returns
/// `OTEL_STATUS_TIMEOUT` if it does not finish in time.
///
/// # Safety
/// `sdk` must satisfy the handle contract and must not be destroyed concurrently.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_force_flush(
    sdk: *mut OtelSdk,
    timeout_millis: u64,
) -> OtelStatus {
    guard_status(|| {
        clear_last_error();
        let sdk = match unsafe { checked_ref::<OtelSdk>(sdk) } {
            Some(s) => s,
            None => return OtelStatus::InvalidArgument,
        };
        if sdk.shutdown.load(Ordering::Acquire) {
            return fail(
                OtelStatus::AlreadyShutdown,
                "cannot force flush a shut-down SDK",
            );
        }
        let timeout = match optional_millis(timeout_millis) {
            None => return map_flush_result(sdk.provider.force_flush()),
            Some(t) => t,
        };
        if sdk.flush_in_flight.swap(true, Ordering::AcqRel) {
            return fail(
                OtelStatus::Timeout,
                "a timed force flush is already in progress; retry after it completes",
            );
        }
        let provider = sdk.provider.clone();
        let guard = FlushGuard(Arc::clone(&sdk.flush_in_flight));
        let (tx, rx) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("otel-c-force-flush".to_owned())
            .spawn(move || {
                let result = provider.force_flush();
                drop(guard);
                let _ = tx.send(result);
            });
        if let Err(err) = spawned {
            sdk.flush_in_flight.store(false, Ordering::Release);
            return fail_owned(
                OtelStatus::InternalError,
                format!("failed to spawn force-flush helper thread: {err}"),
            );
        }
        match rx.recv_timeout(timeout) {
            Ok(result) => map_flush_result(result),
            Err(_) => fail(
                OtelStatus::Timeout,
                "force flush did not complete within the requested timeout",
            ),
        }
    })
}

/// Shut down the SDK, flushing and stopping the pipeline. Runs at most once.
///
/// # Safety
/// `sdk` must satisfy the handle contract and must not be destroyed concurrently.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_shutdown(sdk: *mut OtelSdk, timeout_millis: u64) -> OtelStatus {
    guard_status(|| {
        clear_last_error();
        let sdk = match unsafe { checked_ref::<OtelSdk>(sdk) } {
            Some(s) => s,
            None => return OtelStatus::InvalidArgument,
        };
        if sdk.shutdown.swap(true, Ordering::AcqRel) {
            return fail(
                OtelStatus::AlreadyShutdown,
                "SDK has already been shut down",
            );
        }
        let timeout = optional_millis(timeout_millis).unwrap_or_else(|| Duration::from_secs(5));
        match sdk.provider.shutdown_with_timeout(timeout) {
            Ok(()) => OtelStatus::Ok,
            Err(err) => status_from_sdk_error(&err),
        }
    })
}

/// Destroy an SDK handle (no-op on NULL). Best-effort shutdown on drop.
///
/// # Safety
/// `sdk` must be NULL or a live SDK not used or destroyed concurrently.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_destroy(sdk: *mut OtelSdk) {
    guard_unit(|| unsafe { destroy(sdk) });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch_processor::{
        otel_batch_span_processor_builder_build, otel_batch_span_processor_builder_destroy,
        otel_batch_span_processor_builder_new, otel_batch_span_processor_builder_set_exporter,
    };
    use crate::otlp_exporter::{
        otel_otlp_trace_exporter_builder_build, otel_otlp_trace_exporter_builder_destroy,
        otel_otlp_trace_exporter_builder_new, otel_otlp_trace_exporter_builder_set_endpoint,
    };
    use crate::span_processor::otel_span_processor_destroy;

    fn sv(s: &str) -> OtelStringView {
        OtelStringView {
            ptr: s.as_ptr() as *const std::os::raw::c_char,
            len: s.len(),
        }
    }

    /// Build a real (batch + OTLP) span processor via the pipeline builders, for tests that
    /// need a live `otel_span_processor_t`.
    fn build_processor() -> *mut OtelSpanProcessor {
        unsafe {
            let eb = otel_otlp_trace_exporter_builder_new();
            assert_eq!(
                otel_otlp_trace_exporter_builder_set_endpoint(
                    eb,
                    sv("http://127.0.0.1:9/v1/traces")
                ),
                OtelStatus::Ok
            );
            let mut exporter = std::ptr::null_mut();
            assert_eq!(
                otel_otlp_trace_exporter_builder_build(eb, &mut exporter),
                OtelStatus::Ok
            );
            otel_otlp_trace_exporter_builder_destroy(eb);
            let pb = otel_batch_span_processor_builder_new();
            assert_eq!(
                otel_batch_span_processor_builder_set_exporter(pb, exporter),
                OtelStatus::Ok
            );
            let mut processor = std::ptr::null_mut();
            assert_eq!(
                otel_batch_span_processor_builder_build(pb, &mut processor),
                OtelStatus::Ok
            );
            otel_batch_span_processor_builder_destroy(pb);
            assert!(!processor.is_null());
            processor
        }
    }

    #[test]
    fn set_as_global_registers_sdk_vtable_with_api() {
        // Prove the SDK installs *its* vtable + a non-null provider context into the API's
        // registration ABI (stubbed in unit tests; exercised for real by the cross-artifact
        // C test).
        unsafe {
            let processor = build_processor();
            let b = otel_sdk_builder_new();
            assert_eq!(
                otel_sdk_builder_set_service_name(b, sv("unit-test")),
                OtelStatus::Ok
            );
            assert_eq!(
                otel_sdk_builder_add_span_processor(b, processor),
                OtelStatus::Ok
            );
            let mut sdk: *mut OtelSdk = std::ptr::null_mut();
            assert_eq!(otel_sdk_build(b, &mut sdk), OtelStatus::Ok);
            otel_sdk_builder_destroy(b);

            assert_eq!(otel_sdk_set_as_global(sdk), OtelStatus::Ok);
            let (vtable, ctx) =
                crate::api_ffi::test_probe::registered().expect("SDK must register a provider");
            assert_eq!(vtable, crate::vtable::vtable_ptr());
            assert!(!ctx.is_null());
            // Free the context we handed to the (stub) API to avoid a leak in the test.
            (crate::vtable::SDK_VTABLE.provider_free)(ctx);

            otel_sdk_shutdown(sdk, 500);
            otel_sdk_destroy(sdk);
        }
    }

    #[test]
    fn add_span_processor_ownership_transfer() {
        unsafe {
            // Failure: a bad (NULL) builder leaves the processor caller-owned, so we can still
            // destroy it without a leak/double-free.
            let processor = build_processor();
            assert_eq!(
                otel_sdk_builder_add_span_processor(std::ptr::null_mut(), processor),
                OtelStatus::InvalidArgument
            );
            otel_span_processor_destroy(processor); // still owned by caller: frees it

            // Success: ownership transfers into the SDK builder. A subsequent destroy of the
            // transferred handle is a safe no-op (poisoned), and destroying the builder frees
            // the processor exactly once.
            let processor = build_processor();
            let b = otel_sdk_builder_new();
            assert_eq!(
                otel_sdk_builder_add_span_processor(b, processor),
                OtelStatus::Ok
            );
            otel_span_processor_destroy(processor); // no-op: builder owns it now
            otel_sdk_builder_destroy(b); // frees the transferred processor
        }
    }

    #[test]
    fn build_with_no_processor_succeeds() {
        // A provider with no span processor is valid (spans are simply not exported).
        unsafe {
            let b = otel_sdk_builder_new();
            let mut sdk: *mut OtelSdk = std::ptr::null_mut();
            assert_eq!(otel_sdk_build(b, &mut sdk), OtelStatus::Ok);
            assert!(!sdk.is_null());
            otel_sdk_builder_destroy(b);
            otel_sdk_shutdown(sdk, 500);
            otel_sdk_destroy(sdk);
        }
    }

    #[test]
    fn flush_guard_clears_on_panic() {
        let flag = Arc::new(AtomicBool::new(true));
        let inner = Arc::clone(&flag);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _g = FlushGuard(inner);
            panic!("boom (expected)");
        }));
        assert!(r.is_err());
        assert!(!flag.load(Ordering::Acquire));
    }
}
