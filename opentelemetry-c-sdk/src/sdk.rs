//! SDK builder and lifecycle: configure a resource, an OTLP HTTP/protobuf exporter, and a
//! batch span processor, then build an [`OtelSdk`] that owns the resulting
//! `SdkTracerProvider`. Installing as global (or fetching a provider handle) registers the
//! SDK's implementation into the **API cdylib's** global slot across the C ABI.

use std::collections::HashMap;
use std::os::raw::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::trace::{BatchConfigBuilder, BatchSpanProcessor, SdkTracerProvider};
use opentelemetry_sdk::Resource;

use opentelemetry_c_abi::{OtelKeyValue, OtelStatus, OtelStringView};

use crate::api_ffi;
use crate::error::{clear_last_error, fail, fail_owned, status_from_sdk_error};
use crate::handle::{
    checked_mut, checked_ref, destroy, guard_ptr, guard_status, guard_unit, into_raw, HasMagic,
};
use crate::vtable;

const SDK_BUILDER_MAGIC: u64 = 0x4F54_4C43_5344_4B42; // "OTLCSDKB"
const SDK_MAGIC: u64 = 0x4F54_4C43_5344_4B00; // "OTLCSDK\0"

/// Upper bound on the batch max queue size accepted from C (the processor preallocates a
/// bounded channel of this capacity). `0` selects the SDK default; larger is rejected.
const MAX_BATCH_QUEUE_SIZE: usize = 1_048_576;
/// Upper bound on the batch max export batch size accepted from C (preallocated `Vec`).
const MAX_BATCH_EXPORT_BATCH_SIZE: usize = 1_048_576;

#[derive(Default)]
struct SdkConfig {
    service_name: Option<String>,
    resource_attributes: Vec<KeyValue>,
    otlp_endpoint: Option<String>,
    otlp_headers: Vec<(String, String)>,
    otlp_timeout: Option<Duration>,
    batch_max_queue_size: Option<usize>,
    batch_scheduled_delay: Option<Duration>,
    batch_max_export_batch_size: Option<usize>,
    batch_export_timeout: Option<Duration>,
}

/// Opaque builder handle (`otel_sdk_builder_t`). Not thread-safe; confine to one thread.
pub struct OtelSdkBuilder {
    magic: u64,
    config: SdkConfig,
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
fn optional_size(value: usize) -> Option<usize> {
    (value != 0).then_some(value)
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
    Ok(optional_size(value))
}

// ---- Builder lifecycle -----------------------------------------------------

/// Create a new SDK builder with spec-default settings. Release with `otel_sdk_builder_destroy()`.
#[no_mangle]
pub extern "C" fn otel_sdk_builder_new() -> *mut OtelSdkBuilder {
    guard_ptr(|| {
        clear_last_error();
        into_raw(OtelSdkBuilder {
            magic: SDK_BUILDER_MAGIC,
            config: SdkConfig::default(),
        })
    })
}

/// Destroy an SDK builder (no-op on NULL).
///
/// # Safety
/// `builder` must be NULL or a live builder not destroyed concurrently.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_builder_destroy(builder: *mut OtelSdkBuilder) {
    guard_unit(|| unsafe { destroy(builder) });
}

/// # Safety
/// `builder` must satisfy the handle contract (single-threaded).
unsafe fn with_config<F>(builder: *mut OtelSdkBuilder, f: F) -> OtelStatus
where
    F: FnOnce(&mut SdkConfig) -> OtelStatus,
{
    guard_status(|| {
        clear_last_error();
        match unsafe { checked_mut(builder) } {
            Some(b) => f(&mut b.config),
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
        with_config(builder, |config| match name.to_string_strict() {
            Ok(name) => {
                config.service_name = Some(name);
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
        with_config(builder, |config| match vtable_to_key_value(&attribute) {
            Ok(kv) => {
                config.resource_attributes.push(kv);
                OtelStatus::Ok
            }
            Err(status) => status,
        })
    }
}

/// Set the full OTLP traces endpoint URL, used as-is.
///
/// # Safety
/// `builder` and `endpoint` must satisfy their contracts.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_builder_set_otlp_endpoint(
    builder: *mut OtelSdkBuilder,
    endpoint: OtelStringView,
) -> OtelStatus {
    unsafe {
        with_config(builder, |config| match endpoint.to_string_strict() {
            Ok(endpoint) => {
                config.otlp_endpoint = Some(endpoint);
                OtelStatus::Ok
            }
            Err(e) => crate::error::fail_abi(e),
        })
    }
}

/// Add an HTTP header sent with every OTLP export request.
///
/// # Safety
/// `builder`, `key`, `value` must satisfy their contracts.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_builder_add_otlp_header(
    builder: *mut OtelSdkBuilder,
    key: OtelStringView,
    value: OtelStringView,
) -> OtelStatus {
    unsafe {
        with_config(builder, |config| {
            let key = match key.to_string_strict() {
                Ok(k) if !k.is_empty() => k,
                Ok(_) => {
                    return fail(
                        OtelStatus::InvalidArgument,
                        "OTLP header key must not be empty",
                    )
                }
                Err(e) => return crate::error::fail_abi(e),
            };
            let value = match value.to_string_strict() {
                Ok(v) => v,
                Err(e) => return crate::error::fail_abi(e),
            };
            config.otlp_headers.push((key, value));
            OtelStatus::Ok
        })
    }
}

/// Set the OTLP export request timeout in milliseconds (`0` == exporter default).
///
/// # Safety
/// `builder` must satisfy the handle contract.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_builder_set_otlp_timeout_millis(
    builder: *mut OtelSdkBuilder,
    timeout_millis: u64,
) -> OtelStatus {
    unsafe {
        with_config(builder, |config| {
            config.otlp_timeout = optional_millis(timeout_millis);
            OtelStatus::Ok
        })
    }
}

/// Set the batch processor maximum queue size (`0` == spec default of 2048). An oversized
/// non-zero value is rejected with `OTEL_STATUS_INVALID_ARGUMENT` (never clamped).
///
/// # Safety
/// `builder` must satisfy the handle contract.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_builder_set_batch_max_queue_size(
    builder: *mut OtelSdkBuilder,
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
                    config.batch_max_queue_size = size;
                    OtelStatus::Ok
                }
                Err(status) => status,
            }
        })
    }
}

/// Set the batch processor scheduled delay in milliseconds (`0` == spec default).
///
/// # Safety
/// `builder` must satisfy the handle contract.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_builder_set_batch_scheduled_delay_millis(
    builder: *mut OtelSdkBuilder,
    delay_millis: u64,
) -> OtelStatus {
    unsafe {
        with_config(builder, |config| {
            config.batch_scheduled_delay = optional_millis(delay_millis);
            OtelStatus::Ok
        })
    }
}

/// Set the batch processor maximum export batch size (`0` == spec default of 512).
/// Oversized values are rejected; the effective value is also capped by the SDK at the
/// max queue size.
///
/// # Safety
/// `builder` must satisfy the handle contract.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_builder_set_batch_max_export_batch_size(
    builder: *mut OtelSdkBuilder,
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
                    config.batch_max_export_batch_size = size;
                    OtelStatus::Ok
                }
                Err(status) => status,
            }
        })
    }
}

/// Set the batch processor per-export timeout in milliseconds (`0` == spec default).
///
/// # Safety
/// `builder` must satisfy the handle contract.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_builder_set_batch_export_timeout_millis(
    builder: *mut OtelSdkBuilder,
    timeout_millis: u64,
) -> OtelStatus {
    unsafe {
        with_config(builder, |config| {
            config.batch_export_timeout = optional_millis(timeout_millis);
            OtelStatus::Ok
        })
    }
}

// ---- Build -----------------------------------------------------------------

/// Convert a C attribute into an owned `KeyValue` (used for resource attributes).
fn vtable_to_key_value(kv: &OtelKeyValue) -> Result<KeyValue, OtelStatus> {
    // SAFETY: the builder attribute satisfies the OtelKeyValue string contract.
    unsafe { crate::vtable::to_key_value(kv) }
}

fn build_resource(config: &SdkConfig) -> Resource {
    let mut builder = Resource::builder();
    if let Some(name) = &config.service_name {
        builder = builder.with_service_name(name.clone());
    }
    if !config.resource_attributes.is_empty() {
        builder = builder.with_attributes(config.resource_attributes.iter().cloned());
    }
    builder.build()
}

fn build_exporter(config: &SdkConfig) -> Result<SpanExporter, OtelStatus> {
    let mut builder = SpanExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary);
    if let Some(endpoint) = &config.otlp_endpoint {
        builder = builder.with_endpoint(endpoint.clone());
    }
    if let Some(timeout) = config.otlp_timeout.or(config.batch_export_timeout) {
        builder = builder.with_timeout(timeout);
    }
    if !config.otlp_headers.is_empty() {
        let headers: HashMap<String, String> = config.otlp_headers.iter().cloned().collect();
        builder = builder.with_headers(headers);
    }
    builder.build().map_err(|err| {
        fail_owned(
            OtelStatus::InvalidConfig,
            format!("failed to build OTLP exporter: {err}"),
        )
    })
}

fn build_processor(config: &SdkConfig, exporter: SpanExporter) -> BatchSpanProcessor {
    let mut batch = BatchConfigBuilder::default();
    if let Some(size) = config.batch_max_queue_size {
        batch = batch.with_max_queue_size(size);
    }
    if let Some(delay) = config.batch_scheduled_delay {
        batch = batch.with_scheduled_delay(delay);
    }
    if let Some(size) = config.batch_max_export_batch_size {
        batch = batch.with_max_export_batch_size(size);
    }
    BatchSpanProcessor::builder(exporter)
        .with_batch_config(batch.build())
        .build()
}

/// Build an SDK from the accumulated builder configuration.
///
/// # Safety
/// `builder` must satisfy the handle contract; `out_sdk` a valid writable `otel_sdk_t*`.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_build(
    builder: *const OtelSdkBuilder,
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
        let builder = match unsafe { checked_ref(builder) } {
            Some(b) => b,
            None => return OtelStatus::InvalidArgument,
        };
        let config = &builder.config;
        let exporter = match build_exporter(config) {
            Ok(e) => e,
            Err(status) => return status,
        };
        let processor = build_processor(config, exporter);
        let resource = build_resource(config);
        let provider = SdkTracerProvider::builder()
            .with_span_processor(processor)
            .with_resource(resource)
            .build();
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

    fn sv(s: &str) -> OtelStringView {
        OtelStringView {
            ptr: s.as_ptr() as *const std::os::raw::c_char,
            len: s.len(),
        }
    }

    #[test]
    fn set_as_global_registers_sdk_vtable_with_api() {
        // Prove the SDK installs *its* vtable + a non-null provider context into the API's
        // registration ABI (stubbed in unit tests; exercised for real by the cross-artifact
        // C test).
        unsafe {
            let b = otel_sdk_builder_new();
            assert_eq!(
                otel_sdk_builder_set_otlp_endpoint(b, sv("http://127.0.0.1:9/v1/traces")),
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
    fn oversized_batch_sizes_are_rejected() {
        unsafe {
            let b = otel_sdk_builder_new();
            assert_eq!(
                otel_sdk_builder_set_batch_max_queue_size(b, 0),
                OtelStatus::Ok
            );
            assert_eq!(
                otel_sdk_builder_set_batch_max_queue_size(b, MAX_BATCH_QUEUE_SIZE),
                OtelStatus::Ok
            );
            assert_eq!(
                otel_sdk_builder_set_batch_max_queue_size(b, MAX_BATCH_QUEUE_SIZE + 1),
                OtelStatus::InvalidArgument
            );
            assert_eq!(
                otel_sdk_builder_set_batch_max_export_batch_size(b, usize::MAX),
                OtelStatus::InvalidArgument
            );
            otel_sdk_builder_destroy(b);
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
