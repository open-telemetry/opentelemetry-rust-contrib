//! SDK builder and lifecycle: configure a resource, an OTLP HTTP/protobuf exporter,
//! and a batch span processor, then build an [`OtelSdk`] that owns the resulting
//! `SdkTracerProvider`.
//!
//! The exporter uses the OTLP HTTP transport with the blocking `reqwest` client, so
//! the SDK owns all of its own threading (a dedicated batch-processor OS thread) and
//! **no user-managed async runtime is required**.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::trace::{BatchConfigBuilder, BatchSpanProcessor, SdkTracerProvider};
use opentelemetry_sdk::Resource;

use crate::common::{OtelKeyValue, OtelStringView};
use crate::error::{clear_last_error, fail, status_from_sdk_error, OtelStatus};
use crate::handle::{
    checked_mut, checked_ref, destroy, guard_ptr, guard_status, guard_unit, into_raw, HasMagic,
};
use crate::trace::{provider_from_sdk, OtelTracerProvider};

const SDK_BUILDER_MAGIC: u64 = 0x4F54_4C43_5344_4B42; // "OTLCSDKB"
const SDK_MAGIC: u64 = 0x4F54_4C43_5344_4B00; // "OTLCSDK\0"

/// Mutable configuration accumulated by the SDK builder before [`otel_sdk_build`].
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

/// Opaque builder handle (`otel_sdk_builder_t`).
///
/// **Not thread-safe.** Builder mutations require unique access (`&mut`), so a single
/// builder handle must be confined to one thread (or externally synchronized). This is
/// the natural usage: build the SDK on one thread, then share the resulting `OtelSdk`.
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

/// Opaque SDK handle (`otel_sdk_t`) owning the configured `SdkTracerProvider`.
///
/// All fields provide interior-mutability or shared-ownership semantics, so every
/// operation other than destruction takes `&OtelSdk`. This is what makes concurrent C
/// calls on the same handle sound: no operation ever forms a `&mut OtelSdk`.
pub struct OtelSdk {
    magic: u64,
    provider: SdkTracerProvider,
    shutdown: AtomicBool,
    /// Guards the timed [`otel_sdk_force_flush`] path so at most one helper thread is in
    /// flight at a time (bounds thread growth if the exporter stalls). Shared with the
    /// helper thread, which clears it when its flush finishes.
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

impl OtelSdk {
    /// Borrow the owned provider (used by `otel_sdk_get_tracer_provider`).
    pub(crate) fn provider(&self) -> &SdkTracerProvider {
        &self.provider
    }
}

// Concurrent C callers share a single SDK by raw pointer, and every non-destroy operation
// forms `&OtelSdk`. That is sound across threads only if `OtelSdk: Sync`. Assert it at
// compile time so that adding a non-`Sync` field (which would silently reintroduce
// concurrency unsoundness) fails the build instead.
const _: () = {
    fn assert_sync<T: Sync>() {}
    let _ = assert_sync::<OtelSdk>;
};

/// Interpret a millisecond count as an optional duration (`0` == unset/default).
fn optional_millis(millis: u64) -> Option<Duration> {
    if millis == 0 {
        None
    } else {
        Some(Duration::from_millis(millis))
    }
}

/// Interpret a size argument as optional (`0` == unset/default).
fn optional_size(value: usize) -> Option<usize> {
    if value == 0 {
        None
    } else {
        Some(value)
    }
}

// ---------------------------------------------------------------------------
// Builder lifecycle
// ---------------------------------------------------------------------------

/// Create a new SDK builder with default (spec-compatible) settings.
///
/// Returns NULL only if allocation fails. The caller owns the returned builder and
/// must release it with [`otel_sdk_builder_destroy`].
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

/// Destroy an SDK builder. Passing NULL is a no-op.
///
/// # Safety
/// `builder` must be NULL or a builder returned by [`otel_sdk_builder_new`] that has
/// not already been destroyed.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_builder_destroy(builder: *mut OtelSdkBuilder) {
    guard_unit(|| unsafe { destroy(builder) });
}

/// Helper: run a closure with a validated `&mut SdkConfig`.
///
/// Takes unique access to the builder, so it is **not** safe to call concurrently on the
/// same builder handle (see [`OtelSdkBuilder`]); confine a builder to one thread.
///
/// # Safety
/// `builder` must satisfy the handle contract and must not be used concurrently from
/// another thread.
unsafe fn with_config<F>(builder: *mut OtelSdkBuilder, f: F) -> OtelStatus
where
    F: FnOnce(&mut SdkConfig) -> OtelStatus,
{
    guard_status(|| {
        clear_last_error();
        // SAFETY: forwarded to the caller's contract.
        let builder = match unsafe { checked_mut(builder) } {
            Some(b) => b,
            None => return OtelStatus::InvalidArgument,
        };
        f(&mut builder.config)
    })
}

/// Set the `service.name` resource attribute.
///
/// # Safety
/// `builder` must satisfy the handle contract; `name` must satisfy the string-view
/// contract.
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
            Err(status) => status,
        })
    }
}

/// Add an arbitrary resource attribute.
///
/// # Safety
/// `builder` must satisfy the handle contract; `attribute` must satisfy the
/// key/value contract.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_builder_add_resource_attribute(
    builder: *mut OtelSdkBuilder,
    attribute: OtelKeyValue,
) -> OtelStatus {
    unsafe {
        with_config(builder, |config| match attribute.to_key_value() {
            Ok(kv) => {
                config.resource_attributes.push(kv);
                OtelStatus::Ok
            }
            Err(status) => status,
        })
    }
}

/// Set the full OTLP traces endpoint URL, used as-is (e.g.
/// `http://localhost:4318/v1/traces`). The `/v1/traces` path is **not** appended
/// automatically, so include it.
///
/// If unset, the exporter falls back to the `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`
/// environment variable (used as-is), then `OTEL_EXPORTER_OTLP_ENDPOINT` (with
/// `/v1/traces` appended), then the OTLP default. Programmatic configuration takes
/// precedence over the environment variables.
///
/// # Safety
/// `builder` must satisfy the handle contract; `endpoint` must satisfy the string-view
/// contract.
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
            Err(status) => status,
        })
    }
}

/// Add an HTTP header sent with every OTLP export request (e.g. for authentication).
///
/// # Safety
/// `builder` must satisfy the handle contract; `key` and `value` must satisfy the
/// string-view contract.
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
                Err(status) => return status,
            };
            let value = match value.to_string_strict() {
                Ok(v) => v,
                Err(status) => return status,
            };
            config.otlp_headers.push((key, value));
            OtelStatus::Ok
        })
    }
}

/// Set the OTLP export request timeout in milliseconds (`0` == exporter default).
///
/// This bounds each HTTP export request. It takes precedence over
/// [`otel_sdk_builder_set_batch_export_timeout_millis`].
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

/// Set the batch processor maximum queue size (`0` == spec default of 2048).
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
            config.batch_max_queue_size = optional_size(max_queue_size);
            OtelStatus::Ok
        })
    }
}

/// Set the batch processor scheduled delay in milliseconds (`0` == spec default of 5000).
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
            config.batch_max_export_batch_size = optional_size(max_export_batch_size);
            OtelStatus::Ok
        })
    }
}

/// Set the batch processor per-export timeout in milliseconds (`0` == spec default of 30000).
///
/// Note: with the synchronous batch span processor this crate uses, the effective
/// per-export bound is the OTLP HTTP request timeout. This value is therefore applied
/// as the exporter request timeout unless [`otel_sdk_builder_set_otlp_timeout_millis`]
/// is also set, in which case the explicit OTLP timeout wins.
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

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

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
    // With the synchronous (dedicated-thread) batch processor there is no separate
    // per-export timeout knob; the effective bound is the HTTP request timeout. Honor
    // an explicit OTLP timeout first, then fall back to the batch export timeout.
    if let Some(timeout) = config.otlp_timeout.or(config.batch_export_timeout) {
        builder = builder.with_timeout(timeout);
    }
    if !config.otlp_headers.is_empty() {
        let headers: HashMap<String, String> = config.otlp_headers.iter().cloned().collect();
        builder = builder.with_headers(headers);
    }
    builder.build().map_err(|err| {
        fail(
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
    // `batch_export_timeout` is applied to the exporter (see `build_exporter`) because
    // the synchronous batch processor does not expose a distinct export-timeout setter.
    BatchSpanProcessor::builder(exporter)
        .with_batch_config(batch.build())
        .build()
}

/// Build an SDK from the accumulated builder configuration.
///
/// On success writes a non-NULL SDK handle into `*out_sdk` and returns
/// `OTEL_STATUS_OK`. On failure returns an error status, sets `*out_sdk` to NULL, and
/// records a diagnostic retrievable via `otel_last_error_message()`.
///
/// The builder is only read (not consumed); the caller must still release it with
/// [`otel_sdk_builder_destroy`].
///
/// # Safety
/// `builder` must satisfy the handle contract; `out_sdk` must be a valid, writable
/// pointer to an `otel_sdk_t*`.
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
        // SAFETY: validated non-NULL above.
        unsafe { *out_sdk = std::ptr::null_mut() };

        // SAFETY: forwarded to the caller's contract.
        let builder = match unsafe { checked_ref(builder) } {
            Some(b) => b,
            None => return OtelStatus::InvalidArgument,
        };
        let config = &builder.config;

        let exporter = match build_exporter(config) {
            Ok(exporter) => exporter,
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
        // SAFETY: validated non-NULL above.
        unsafe { *out_sdk = sdk };
        OtelStatus::Ok
    })
}

// ---------------------------------------------------------------------------
// Provider access and global installation
// ---------------------------------------------------------------------------

/// Return an owned tracer-provider handle backed by this SDK.
///
/// The returned handle is independent of the SDK handle's lifetime (it holds its own
/// reference to the underlying provider) and must be released with
/// `otel_tracer_provider_destroy`. Returns NULL if the SDK handle is invalid.
///
/// # Safety
/// `sdk` must satisfy the handle contract.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_get_tracer_provider(
    sdk: *const OtelSdk,
) -> *mut OtelTracerProvider {
    guard_ptr(|| {
        clear_last_error();
        // SAFETY: forwarded to the caller's contract.
        match unsafe { checked_ref(sdk) } {
            Some(sdk) => provider_from_sdk(sdk.provider().clone()),
            None => std::ptr::null_mut(),
        }
    })
}

/// Install this SDK's tracer provider as the process-global provider.
///
/// After this call, `otel_global_tracer_provider()` and tracers obtained from it emit
/// through this SDK. May be called more than once; the most recent call wins. Returns
/// `OTEL_STATUS_ALREADY_SHUTDOWN` if the SDK has been shut down.
///
/// Safe to call concurrently with other non-destroy SDK operations on the same handle. A
/// concurrent `set_as_global` and [`otel_sdk_shutdown`] may linearize in **either order**,
/// and both outcomes are acceptable: if `set_as_global` observes the SDK as not-yet-shut-
/// down it may publish the provider even if `shutdown` is about to win (the just-published
/// provider then simply becomes a no-op once shutdown completes); once shutdown has been
/// observed, `set_as_global` returns `OTEL_STATUS_ALREADY_SHUTDOWN`. No locking is needed.
///
/// # Safety
/// `sdk` must satisfy the handle contract and must not be destroyed concurrently.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_set_as_global(sdk: *mut OtelSdk) -> OtelStatus {
    guard_status(|| {
        clear_last_error();
        // SAFETY: forwarded to the caller's contract. A shared borrow is sufficient (the
        // provider clone and shutdown flag are both interior-mutable), so concurrent
        // callers never alias a `&mut`.
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
        opentelemetry::global::set_tracer_provider(sdk.provider.clone());
        OtelStatus::Ok
    })
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

fn map_flush_result(result: opentelemetry_sdk::error::OTelSdkResult) -> OtelStatus {
    match result {
        Ok(()) => OtelStatus::Ok,
        Err(err) => status_from_sdk_error(&err),
    }
}

/// Clears the shared force-flush in-flight flag when dropped. Held by the helper thread
/// so the flag is released even if `provider.force_flush()` panics (which would otherwise
/// leave the flag set and block every future timed flush).
struct FlushGuard(Arc<AtomicBool>);

impl Drop for FlushGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Flush any buffered spans to the exporter.
///
/// If `timeout_millis` is `0` the call blocks on the calling thread until the flush
/// completes. If `timeout_millis` is non-zero the flush runs on a helper thread and the
/// call returns `OTEL_STATUS_TIMEOUT` if it has not completed in time (the flush
/// continues in the background).
///
/// Threading contract for the timed path: **at most one timed force-flush helper thread
/// exists at a time**. If a timed flush is already in progress on another thread (for
/// example the exporter stalled and a prior timed flush returned `OTEL_STATUS_TIMEOUT`
/// while its helper kept running), a concurrent timed flush returns
/// `OTEL_STATUS_TIMEOUT` immediately rather than spawning another thread. This bounds
/// helper-thread growth. A blocking flush (`timeout_millis == 0`) never spawns a thread
/// and is not subject to this guard. If the helper thread cannot be spawned,
/// `OTEL_STATUS_INTERNAL_ERROR` is returned.
///
/// Safe to call concurrently with other non-destroy SDK operations on the same handle.
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
        // SAFETY: forwarded to the caller's contract. A shared borrow is sufficient; the
        // provider is `&self` and the flags are atomics, so concurrent callers are sound.
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
            Some(timeout) => timeout,
        };

        // Timed flush. Bound helper-thread growth: only one timed flush may be in flight
        // at a time. If one already is, do not spawn another.
        if sdk.flush_in_flight.swap(true, Ordering::AcqRel) {
            return fail(
                OtelStatus::Timeout,
                "a timed force flush is already in progress; retry after it completes",
            );
        }

        // Run the flush on a helper thread so the C-supplied timeout is honored even if
        // the exporter stalls. The provider is cheaply cloneable (Arc-backed); a
        // `FlushGuard` clears the shared in-flight flag when the helper finishes, even if
        // the flush panics.
        let provider = sdk.provider.clone();
        let guard = FlushGuard(Arc::clone(&sdk.flush_in_flight));
        let (tx, rx) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("otel-c-force-flush".to_owned())
            .spawn(move || {
                let result = provider.force_flush();
                // Release the guard now (before sending) so a subsequent sequential flush
                // is not spuriously rejected; on a panic the guard's `Drop` clears it.
                drop(guard);
                let _ = tx.send(result);
            });
        if let Err(err) = spawned {
            // The helper never started, so clear the in-flight flag here.
            sdk.flush_in_flight.store(false, Ordering::Release);
            return fail(
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

/// Shut down the SDK, flushing and stopping the exporter pipeline.
///
/// The underlying provider shutdown runs **at most once**: the first call performs it;
/// concurrent or subsequent calls return `OTEL_STATUS_ALREADY_SHUTDOWN` without touching
/// the provider. `timeout_millis` of `0` uses the SDK default (5s). After shutdown, span
/// creation through this SDK's provider becomes a no-op.
///
/// Safe to call concurrently with other non-destroy SDK operations on the same handle;
/// a concurrent `otel_sdk_force_flush` either observes the shutdown (and returns
/// `OTEL_STATUS_ALREADY_SHUTDOWN`) or completes against the still-live provider.
///
/// # Safety
/// `sdk` must satisfy the handle contract and must not be destroyed concurrently.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_shutdown(sdk: *mut OtelSdk, timeout_millis: u64) -> OtelStatus {
    guard_status(|| {
        clear_last_error();
        // SAFETY: forwarded to the caller's contract. A shared borrow is sufficient: the
        // once-guard is an atomic swap and provider shutdown takes `&self`.
        let sdk = match unsafe { checked_ref::<OtelSdk>(sdk) } {
            Some(s) => s,
            None => return OtelStatus::InvalidArgument,
        };
        // Ensure the underlying provider shutdown runs at most once, even under a race.
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

/// Destroy an SDK handle. Passing NULL is a no-op.
///
/// If the SDK was not explicitly shut down, dropping it triggers a best-effort
/// shutdown of the owned provider. Callers should still call [`otel_sdk_shutdown`]
/// explicitly to observe the result and bound the flush time.
///
/// # Safety
/// `sdk` must be NULL or an SDK returned by [`otel_sdk_build`] that has not already
/// been destroyed.
#[no_mangle]
pub unsafe extern "C" fn otel_sdk_destroy(sdk: *mut OtelSdk) {
    guard_unit(|| unsafe { destroy(sdk) });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flush_guard_clears_flag_on_normal_drop() {
        let flag = Arc::new(AtomicBool::new(true));
        {
            let _guard = FlushGuard(Arc::clone(&flag));
        }
        assert!(
            !flag.load(Ordering::Acquire),
            "FlushGuard::drop must clear the in-flight flag"
        );
    }

    #[test]
    fn flush_guard_clears_flag_even_on_panic() {
        // If the flush panics inside the helper thread, the guard's Drop must still clear
        // the flag; otherwise every future timed flush would be wrongly rejected.
        let flag = Arc::new(AtomicBool::new(true));
        let inner = Arc::clone(&flag);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _guard = FlushGuard(inner);
            panic!("simulated force_flush panic (expected in this test)");
        }));
        assert!(result.is_err(), "the closure must have panicked");
        assert!(
            !flag.load(Ordering::Acquire),
            "FlushGuard::drop must clear the in-flight flag even on panic"
        );
    }
}
