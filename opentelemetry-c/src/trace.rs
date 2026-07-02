//! Trace API surface: tracer providers, tracers, and spans exposed as opaque handles.
//!
//! Handles wrap the OpenTelemetry object-safe types ([`BoxedTracer`] / [`BoxedSpan`])
//! so a single ABI works for both SDK-backed and global providers.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use opentelemetry::global::{BoxedSpan, BoxedTracer};
use opentelemetry::trace::{
    Span, SpanContext, SpanKind, Status, TraceContextExt, Tracer, TracerProvider,
};
use opentelemetry::{Context, InstrumentationScope, KeyValue};
use opentelemetry_sdk::trace::SdkTracerProvider;

use crate::common::{OtelBool, OtelKeyValue, OtelStringView};
use crate::error::{clear_last_error, fail, OtelStatus};
use crate::handle::{
    checked_mut, checked_ref, destroy, guard_ptr, guard_status, guard_unit, into_raw, HasMagic,
};

const PROVIDER_MAGIC: u64 = 0x4F54_4C43_5052_4F56; // "OTLCPROV"
const TRACER_MAGIC: u64 = 0x4F54_4C43_5452_4143; // "OTLCTRAC"
const SPAN_MAGIC: u64 = 0x4F54_4C43_5350_414E; // "OTLCSPAN"

/// Backing implementation for an [`OtelTracerProvider`] handle.
enum ProviderInner {
    /// A provider owned by a specific SDK instance.
    Sdk(SdkTracerProvider),
    /// The process-global provider (resolved lazily on each tracer request).
    Global,
}

/// Opaque tracer-provider handle (`otel_tracer_provider_t`).
pub struct OtelTracerProvider {
    magic: u64,
    inner: ProviderInner,
}

impl HasMagic for OtelTracerProvider {
    const MAGIC: u64 = PROVIDER_MAGIC;
    fn magic(&self) -> u64 {
        self.magic
    }
    fn set_magic(&mut self, value: u64) {
        self.magic = value;
    }
}

/// Construct an owned SDK-backed provider handle. Used by the SDK module.
pub(crate) fn provider_from_sdk(provider: SdkTracerProvider) -> *mut OtelTracerProvider {
    into_raw(OtelTracerProvider {
        magic: PROVIDER_MAGIC,
        inner: ProviderInner::Sdk(provider),
    })
}

/// Opaque tracer handle (`otel_tracer_t`).
pub struct OtelTracer {
    magic: u64,
    tracer: BoxedTracer,
}

impl HasMagic for OtelTracer {
    const MAGIC: u64 = TRACER_MAGIC;
    fn magic(&self) -> u64 {
        self.magic
    }
    fn set_magic(&mut self, value: u64) {
        self.magic = value;
    }
}

/// Opaque span handle (`otel_span_t`).
pub struct OtelSpan {
    magic: u64,
    span: BoxedSpan,
    ended: AtomicBool,
}

impl HasMagic for OtelSpan {
    const MAGIC: u64 = SPAN_MAGIC;
    fn magic(&self) -> u64 {
        self.magic
    }
    fn set_magic(&mut self, value: u64) {
        self.magic = value;
    }
}

/// Span kind, mirroring `opentelemetry::trace::SpanKind`.
///
/// Crosses the C boundary as a raw `u32` (see [`OtelSpanStartOptions::kind`]); this
/// enum documents the valid values and validates them via [`Self::from_u32`].
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtelSpanKind {
    /// Default: internal operation with no remote counterpart.
    Internal = 0,
    /// Handles a synchronous inbound request from a remote peer.
    Server = 1,
    /// Issues a synchronous outbound request to a remote peer.
    Client = 2,
    /// Initiates an asynchronous message to be handled later.
    Producer = 3,
    /// Processes an asynchronous message from a producer.
    Consumer = 4,
}

impl OtelSpanKind {
    /// Validate a raw span-kind value received from C, returning `None` if unknown.
    pub(crate) fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(OtelSpanKind::Internal),
            1 => Some(OtelSpanKind::Server),
            2 => Some(OtelSpanKind::Client),
            3 => Some(OtelSpanKind::Producer),
            4 => Some(OtelSpanKind::Consumer),
            _ => None,
        }
    }
}

impl From<OtelSpanKind> for SpanKind {
    fn from(kind: OtelSpanKind) -> Self {
        match kind {
            OtelSpanKind::Internal => SpanKind::Internal,
            OtelSpanKind::Server => SpanKind::Server,
            OtelSpanKind::Client => SpanKind::Client,
            OtelSpanKind::Producer => SpanKind::Producer,
            OtelSpanKind::Consumer => SpanKind::Consumer,
        }
    }
}

/// Span status code, mirroring `opentelemetry::trace::Status`.
///
/// Crosses the C boundary as a raw `u32` (see [`otel_span_set_status`]); this enum
/// documents the valid values and validates them via [`Self::from_u32`].
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtelSpanStatusCode {
    /// Default, unset status.
    Unset = 0,
    /// Explicitly marked successful by the application.
    Ok = 1,
    /// The operation contains an error (see the description argument).
    Error = 2,
}

impl OtelSpanStatusCode {
    /// Validate a raw status-code value received from C, returning `None` if unknown.
    pub(crate) fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(OtelSpanStatusCode::Unset),
            1 => Some(OtelSpanStatusCode::Ok),
            2 => Some(OtelSpanStatusCode::Error),
            _ => None,
        }
    }
}

/// Options for [`otel_tracer_start_span`]. All fields are optional; a NULL options
/// pointer selects `Internal` kind and no explicit parent (a new root span).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct OtelSpanStartOptions {
    /// The span kind, as an [`OtelSpanKind`] value. Unknown values fall back to
    /// `Internal`.
    pub kind: u32,
    /// Optional parent span. When non-NULL the new span becomes its child; when NULL
    /// the new span is a root span. The parent handle is only borrowed for the call.
    pub parent: *const OtelSpan,
}

// Compile-time ABI guard mirroring the C header `_Static_assert`.
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(std::mem::size_of::<OtelSpanStartOptions>() == 16);
    assert!(std::mem::align_of::<OtelSpanStartOptions>() == 8);
};

/// A minimal [`Span`] that carries only a parent's [`SpanContext`].
///
/// It is used to install an in-process parent as the active span of a [`Context`] when
/// starting a child span. Only [`Span::span_context`] is meaningful; every other
/// operation is a no-op and the span never records. The carried context always has
/// `is_remote = false`, so a child built from it records `parent_span_is_remote = false`
/// — i.e. a local parent, not one extracted from remote propagation.
///
/// This exists because the only public APIs that seed a `Context`'s active span from a
/// bare `SpanContext` are `with_span` (used here) and `with_remote_span_context`; the
/// latter is intended for remote parents and would mislabel an in-process parent.
struct LocalParentSpan {
    span_context: SpanContext,
}

impl LocalParentSpan {
    /// Build a local parent from an in-process span's borrowed context, forcing
    /// `is_remote = false`. Trace id, span id, and flags are copied; the trace state is
    /// cloned exactly once (it is the only heap-owning field of a `SpanContext`).
    fn new(parent: &SpanContext) -> Self {
        LocalParentSpan {
            span_context: SpanContext::new(
                parent.trace_id(),
                parent.span_id(),
                parent.trace_flags(),
                false,
                parent.trace_state().clone(),
            ),
        }
    }
}

impl Span for LocalParentSpan {
    fn add_event_with_timestamp<T>(
        &mut self,
        _name: T,
        _timestamp: SystemTime,
        _attributes: Vec<KeyValue>,
    ) where
        T: Into<Cow<'static, str>>,
    {
    }

    fn span_context(&self) -> &SpanContext {
        &self.span_context
    }

    fn is_recording(&self) -> bool {
        false
    }

    fn set_attribute(&mut self, _attribute: KeyValue) {}

    fn set_status(&mut self, _status: Status) {}

    fn update_name<T>(&mut self, _new_name: T)
    where
        T: Into<Cow<'static, str>>,
    {
    }

    fn add_link(&mut self, _span_context: SpanContext, _attributes: Vec<KeyValue>) {}

    fn end_with_timestamp(&mut self, _timestamp: SystemTime) {}
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

/// Return an owned handle to the process-global tracer provider.
///
/// Never NULL under normal conditions. The caller must release it with
/// [`otel_tracer_provider_destroy`]. The handle resolves the global provider lazily,
/// so it reflects whichever SDK is installed at tracer-creation time.
#[no_mangle]
pub extern "C" fn otel_global_tracer_provider() -> *mut OtelTracerProvider {
    guard_ptr(|| {
        clear_last_error();
        into_raw(OtelTracerProvider {
            magic: PROVIDER_MAGIC,
            inner: ProviderInner::Global,
        })
    })
}

/// Obtain a tracer from a provider.
///
/// `name` identifies the instrumentation scope (required). `version` and `schema_url`
/// are optional: pass an empty string view (`len == 0`) to omit them.
///
/// Returns NULL if the provider handle is invalid or an argument is malformed; the
/// returned tracer must be released with [`otel_tracer_destroy`].
///
/// # Safety
/// `provider` must satisfy the handle contract; the string views must satisfy the
/// string-view contract.
#[no_mangle]
pub unsafe extern "C" fn otel_tracer_provider_get_tracer(
    provider: *const OtelTracerProvider,
    name: OtelStringView,
    version: OtelStringView,
    schema_url: OtelStringView,
) -> *mut OtelTracer {
    guard_ptr(|| {
        clear_last_error();
        // SAFETY: forwarded to the caller's contract.
        let provider = match unsafe { checked_ref(provider) } {
            Some(p) => p,
            None => return std::ptr::null_mut(),
        };

        // SAFETY: forwarded to the caller's contract.
        let name = match unsafe { name.to_string_lossy() } {
            Ok(name) => name,
            Err(_) => return std::ptr::null_mut(),
        };

        let mut scope = InstrumentationScope::builder(name);
        // SAFETY: forwarded to the caller's contract.
        match unsafe { version.to_string_lossy() } {
            Ok(v) if !v.is_empty() => scope = scope.with_version(v),
            Ok(_) => {}
            Err(_) => return std::ptr::null_mut(),
        }
        // SAFETY: forwarded to the caller's contract.
        match unsafe { schema_url.to_string_lossy() } {
            Ok(s) if !s.is_empty() => scope = scope.with_schema_url(s),
            Ok(_) => {}
            Err(_) => return std::ptr::null_mut(),
        }
        let scope = scope.build();

        let tracer = match &provider.inner {
            ProviderInner::Sdk(sdk) => BoxedTracer::new(Box::new(sdk.tracer_with_scope(scope))),
            ProviderInner::Global => opentelemetry::global::tracer_with_scope(scope),
        };

        into_raw(OtelTracer {
            magic: TRACER_MAGIC,
            tracer,
        })
    })
}

/// Destroy a tracer-provider handle. Passing NULL is a no-op. Destroying the handle
/// does **not** shut down the underlying SDK.
///
/// # Safety
/// `provider` must be NULL or a provider handle that has not already been destroyed.
#[no_mangle]
pub unsafe extern "C" fn otel_tracer_provider_destroy(provider: *mut OtelTracerProvider) {
    guard_unit(|| unsafe { destroy(provider) });
}

// ---------------------------------------------------------------------------
// Tracer
// ---------------------------------------------------------------------------

/// Start a new span.
///
/// `name` is the span name (required). `options` may be NULL to start a root span with
/// `Internal` kind. Returns NULL if the tracer handle is invalid or the name is
/// malformed; the returned span must be ended and released with [`otel_span_end`] /
/// [`otel_span_destroy`] (destroying without ending performs a best-effort end).
///
/// # Safety
/// `tracer` must satisfy the handle contract; `options` (if non-NULL) must point to a
/// valid [`OtelSpanStartOptions`] whose `parent` is NULL or a valid span handle; the
/// name view must satisfy the string-view contract.
#[no_mangle]
pub unsafe extern "C" fn otel_tracer_start_span(
    tracer: *const OtelTracer,
    name: OtelStringView,
    options: *const OtelSpanStartOptions,
) -> *mut OtelSpan {
    guard_ptr(|| {
        clear_last_error();
        // SAFETY: forwarded to the caller's contract.
        let tracer = match unsafe { checked_ref(tracer) } {
            Some(t) => t,
            None => return std::ptr::null_mut(),
        };
        // SAFETY: forwarded to the caller's contract.
        let name = match unsafe { name.to_string_lossy() } {
            Ok(name) => name,
            Err(_) => return std::ptr::null_mut(),
        };

        let mut kind = SpanKind::Internal;
        let mut local_parent: Option<LocalParentSpan> = None;
        if !options.is_null() {
            // SAFETY: caller guarantees a valid options pointer when non-NULL.
            let options = unsafe { &*options };
            // Unknown span-kind values degrade to `Internal` rather than being rejected.
            kind = OtelSpanKind::from_u32(options.kind)
                .unwrap_or(OtelSpanKind::Internal)
                .into();
            if !options.parent.is_null() {
                // SAFETY: caller guarantees parent is NULL or a valid span handle.
                match unsafe { checked_ref::<OtelSpan>(options.parent) } {
                    // Build the local parent directly from the borrowed span context: this
                    // clones the parent's trace state exactly once and avoids the extra
                    // intermediate full `SpanContext` clone.
                    Some(parent) => {
                        local_parent = Some(LocalParentSpan::new(parent.span.span_context()))
                    }
                    None => return std::ptr::null_mut(),
                }
            }
        }

        let builder = tracer.tracer.span_builder(name).with_kind(kind);
        let span = match local_parent {
            Some(parent) => {
                // The parent is an in-process span handle. Install it as the active span
                // of a fresh context so the child inherits the parent's trace id and
                // records the parent's span id as a *local* (non-remote) parent. We do
                // not use `with_remote_span_context`, which is meant for parents
                // extracted from remote propagation and would mark the parent remote.
                let cx = Context::new().with_span(parent);
                tracer.tracer.build_with_context(builder, &cx)
            }
            // No explicit parent => a root span. The C API is deterministic and does not
            // consult Rust's thread-local current context.
            None => tracer.tracer.build_with_context(builder, &Context::new()),
        };

        into_raw(OtelSpan {
            magic: SPAN_MAGIC,
            span,
            ended: AtomicBool::new(false),
        })
    })
}

/// Destroy a tracer handle. Passing NULL is a no-op.
///
/// # Safety
/// `tracer` must be NULL or a tracer handle that has not already been destroyed.
#[no_mangle]
pub unsafe extern "C" fn otel_tracer_destroy(tracer: *mut OtelTracer) {
    guard_unit(|| unsafe { destroy(tracer) });
}

// ---------------------------------------------------------------------------
// Span
// ---------------------------------------------------------------------------

/// Helper: run a closure with a validated `&mut OtelSpan`.
///
/// Span mutation takes unique access (`&mut`), so a single span handle must **not** be
/// used concurrently from multiple threads (doing so would create `&mut` aliasing and is
/// undefined behavior). This is the documented C contract: one span per thread, or
/// external synchronization. Distinct spans may be used on different threads at once.
///
/// # Safety
/// `span` must satisfy the handle contract and must not be used concurrently from another
/// thread or destroyed concurrently.
unsafe fn with_span<F>(span: *mut OtelSpan, f: F) -> OtelStatus
where
    F: FnOnce(&mut OtelSpan) -> OtelStatus,
{
    guard_status(|| {
        clear_last_error();
        // SAFETY: forwarded to the caller's contract (unique, non-concurrent access).
        match unsafe { checked_mut(span) } {
            Some(span) => f(span),
            None => OtelStatus::InvalidArgument,
        }
    })
}

/// Set a string attribute on a span.
///
/// # Safety
/// `span` must satisfy the handle contract; the string views must satisfy the
/// string-view contract.
#[no_mangle]
pub unsafe extern "C" fn otel_span_set_string_attribute(
    span: *mut OtelSpan,
    key: OtelStringView,
    value: OtelStringView,
) -> OtelStatus {
    unsafe {
        with_span(span, |span| {
            let key = match key.to_string_lossy() {
                Ok(k) if !k.is_empty() => k,
                Ok(_) => {
                    return fail(
                        OtelStatus::InvalidArgument,
                        "attribute key must not be empty",
                    )
                }
                Err(status) => return status,
            };
            let value = match value.to_string_lossy() {
                Ok(v) => v,
                Err(status) => return status,
            };
            span.span
                .set_attribute(opentelemetry::KeyValue::new(key, value));
            OtelStatus::Ok
        })
    }
}

/// Set a boolean attribute on a span.
///
/// `value` follows the C boolean convention: `0` is false, any non-zero value is true.
///
/// # Safety
/// `span` must satisfy the handle contract; `key` must satisfy the string-view
/// contract.
#[no_mangle]
pub unsafe extern "C" fn otel_span_set_bool_attribute(
    span: *mut OtelSpan,
    key: OtelStringView,
    value: OtelBool,
) -> OtelStatus {
    unsafe {
        with_span(span, |span| {
            let key = match key.to_string_lossy() {
                Ok(k) if !k.is_empty() => k,
                Ok(_) => {
                    return fail(
                        OtelStatus::InvalidArgument,
                        "attribute key must not be empty",
                    )
                }
                Err(status) => return status,
            };
            span.span
                .set_attribute(opentelemetry::KeyValue::new(key, value != 0));
            OtelStatus::Ok
        })
    }
}

/// Set a 64-bit signed integer attribute on a span.
///
/// # Safety
/// `span` must satisfy the handle contract; `key` must satisfy the string-view
/// contract.
#[no_mangle]
pub unsafe extern "C" fn otel_span_set_int64_attribute(
    span: *mut OtelSpan,
    key: OtelStringView,
    value: i64,
) -> OtelStatus {
    unsafe {
        with_span(span, |span| {
            let key = match key.to_string_lossy() {
                Ok(k) if !k.is_empty() => k,
                Ok(_) => {
                    return fail(
                        OtelStatus::InvalidArgument,
                        "attribute key must not be empty",
                    )
                }
                Err(status) => return status,
            };
            span.span
                .set_attribute(opentelemetry::KeyValue::new(key, value));
            OtelStatus::Ok
        })
    }
}

/// Set a double-precision floating point attribute on a span.
///
/// # Safety
/// `span` must satisfy the handle contract; `key` must satisfy the string-view
/// contract.
#[no_mangle]
pub unsafe extern "C" fn otel_span_set_double_attribute(
    span: *mut OtelSpan,
    key: OtelStringView,
    value: f64,
) -> OtelStatus {
    unsafe {
        with_span(span, |span| {
            let key = match key.to_string_lossy() {
                Ok(k) if !k.is_empty() => k,
                Ok(_) => {
                    return fail(
                        OtelStatus::InvalidArgument,
                        "attribute key must not be empty",
                    )
                }
                Err(status) => return status,
            };
            span.span
                .set_attribute(opentelemetry::KeyValue::new(key, value));
            OtelStatus::Ok
        })
    }
}

/// Set a typed attribute on a span from an [`OtelKeyValue`].
///
/// # Safety
/// `span` must satisfy the handle contract; `attribute` must satisfy the key/value
/// contract.
#[no_mangle]
pub unsafe extern "C" fn otel_span_set_attribute(
    span: *mut OtelSpan,
    attribute: OtelKeyValue,
) -> OtelStatus {
    unsafe {
        with_span(span, |span| match attribute.to_key_value() {
            Ok(kv) => {
                span.span.set_attribute(kv);
                OtelStatus::Ok
            }
            Err(status) => status,
        })
    }
}

/// Add a timestamped event to a span.
///
/// `attributes` may be NULL when `attribute_count` is 0.
///
/// # Safety
/// `span` must satisfy the handle contract; `name` must satisfy the string-view
/// contract; `attributes` must point to `attribute_count` valid [`OtelKeyValue`]s or
/// be NULL when the count is 0.
#[no_mangle]
pub unsafe extern "C" fn otel_span_add_event(
    span: *mut OtelSpan,
    name: OtelStringView,
    attributes: *const OtelKeyValue,
    attribute_count: usize,
) -> OtelStatus {
    unsafe {
        with_span(span, |span| {
            let name = match name.to_string_lossy() {
                Ok(name) => name,
                Err(status) => return status,
            };
            let attributes = match crate::common::collect_key_values(attributes, attribute_count) {
                Ok(attrs) => attrs,
                Err(status) => return status,
            };
            span.span.add_event(name, attributes);
            OtelStatus::Ok
        })
    }
}

/// Set the status of a span.
///
/// `code` is an [`OtelSpanStatusCode`] value. For `OTEL_SPAN_STATUS_ERROR`,
/// `description` provides the error message; for other codes `description` is ignored
/// and may be an empty view. An unknown `code` is rejected with
/// `OTEL_STATUS_INVALID_ARGUMENT`.
///
/// # Safety
/// `span` must satisfy the handle contract; `description` must satisfy the string-view
/// contract.
#[no_mangle]
pub unsafe extern "C" fn otel_span_set_status(
    span: *mut OtelSpan,
    code: u32,
    description: OtelStringView,
) -> OtelStatus {
    unsafe {
        with_span(span, |span| {
            let code = match OtelSpanStatusCode::from_u32(code) {
                Some(code) => code,
                None => {
                    return fail(
                        OtelStatus::InvalidArgument,
                        "status code is not a valid OtelSpanStatusCode value",
                    )
                }
            };
            let status = match code {
                OtelSpanStatusCode::Unset => Status::Unset,
                OtelSpanStatusCode::Ok => Status::Ok,
                OtelSpanStatusCode::Error => {
                    let description = match description.to_string_lossy() {
                        Ok(d) => d,
                        Err(status) => return status,
                    };
                    Status::error(description)
                }
            };
            span.span.set_status(status);
            OtelStatus::Ok
        })
    }
}

/// Update (rename) a span.
///
/// # Safety
/// `span` must satisfy the handle contract; `name` must satisfy the string-view
/// contract.
#[no_mangle]
pub unsafe extern "C" fn otel_span_update_name(
    span: *mut OtelSpan,
    name: OtelStringView,
) -> OtelStatus {
    unsafe {
        with_span(span, |span| {
            let name = match name.to_string_lossy() {
                Ok(name) => name,
                Err(status) => return status,
            };
            span.span.update_name(name);
            OtelStatus::Ok
        })
    }
}

/// End a span, recording its end timestamp.
///
/// Idempotent: ending a span more than once is safe and subsequent calls return
/// `OTEL_STATUS_OK` without re-ending it.
///
/// # Safety
/// `span` must satisfy the handle contract.
#[no_mangle]
pub unsafe extern "C" fn otel_span_end(span: *mut OtelSpan) -> OtelStatus {
    unsafe {
        with_span(span, |span| {
            if !span.ended.swap(true, Ordering::AcqRel) {
                span.span.end();
            }
            OtelStatus::Ok
        })
    }
}

/// Destroy a span handle. Passing NULL is a no-op.
///
/// If the span was not explicitly ended, dropping the handle performs a best-effort
/// end (recording the current time). Prefer calling [`otel_span_end`] explicitly.
///
/// # Safety
/// `span` must be NULL or a span handle that has not already been destroyed.
#[no_mangle]
pub unsafe extern "C" fn otel_span_destroy(span: *mut OtelSpan) {
    guard_unit(|| unsafe { destroy(span) });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::raw::c_char;
    use std::ptr;

    use opentelemetry::trace::{SpanId, TraceId};
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SimpleSpanProcessor};

    fn sv(s: &str) -> OtelStringView {
        OtelStringView {
            ptr: s.as_ptr() as *const c_char,
            len: s.len(),
        }
    }

    fn empty() -> OtelStringView {
        OtelStringView {
            ptr: ptr::null(),
            len: 0,
        }
    }

    /// Build an SDK-backed provider handle whose spans are exported synchronously into
    /// `exporter` on end (via a `SimpleSpanProcessor`, which needs no async runtime).
    fn provider_with_exporter(exporter: InMemorySpanExporter) -> *mut OtelTracerProvider {
        let sdk = SdkTracerProvider::builder()
            .with_span_processor(SimpleSpanProcessor::new(exporter))
            .build();
        provider_from_sdk(sdk)
    }

    /// A local parent span handle must produce a child that shares the parent's trace,
    /// records the parent's span id as a non-remote parent, and can end independently.
    #[test]
    fn explicit_parent_creates_local_child_span() {
        let exporter = InMemorySpanExporter::default();
        let provider = provider_with_exporter(exporter.clone());
        unsafe {
            let tracer = otel_tracer_provider_get_tracer(provider, sv("scope"), empty(), empty());
            assert!(!tracer.is_null());

            // Root parent span.
            let parent = otel_tracer_start_span(tracer, sv("parent"), ptr::null());
            assert!(!parent.is_null());
            let parent_ctx = (*parent).span.span_context().clone();
            assert!(parent_ctx.is_valid());

            // Child span with an explicit in-process parent handle.
            let opts = OtelSpanStartOptions {
                kind: OtelSpanKind::Client as u32,
                parent: parent as *const OtelSpan,
            };
            let child = otel_tracer_start_span(tracer, sv("child"), &opts);
            assert!(!child.is_null());
            let child_ctx = (*child).span.span_context().clone();

            // The child can be ended before its parent.
            assert_eq!(otel_span_end(child), OtelStatus::Ok);
            assert_eq!(otel_span_end(parent), OtelStatus::Ok);

            // Inspect the exported span data.
            let spans = exporter.get_finished_spans().expect("exported spans");
            let child_data = spans
                .iter()
                .find(|s| s.name == "child")
                .expect("child exported");
            let parent_data = spans
                .iter()
                .find(|s| s.name == "parent")
                .expect("parent exported");

            // Parent and child share the same trace id.
            assert_eq!(
                child_data.span_context.trace_id(),
                parent_ctx.trace_id(),
                "child must inherit the parent's trace id"
            );
            assert_eq!(parent_data.span_context.trace_id(), parent_ctx.trace_id());
            // The child's parent span id equals the parent's span id.
            assert_eq!(
                child_data.parent_span_id,
                parent_ctx.span_id(),
                "child.parent_span_id must equal the parent span id"
            );
            // Sanity: the child's own id matches the handle we observed.
            assert_eq!(child_data.span_context.span_id(), child_ctx.span_id());
            // An in-process parent is NOT remote.
            assert!(
                !child_data.parent_span_is_remote,
                "a local parent must not be flagged remote"
            );
            // The child records the requested kind.
            assert_eq!(child_data.span_kind, SpanKind::Client);

            // The parent is a root span: no parent, not remote.
            assert_eq!(parent_data.parent_span_id, SpanId::INVALID);
            assert!(!parent_data.parent_span_is_remote);

            otel_span_destroy(child);
            otel_span_destroy(parent);
            otel_tracer_destroy(tracer);
            otel_tracer_provider_destroy(provider);
        }
    }

    /// A NULL options pointer (and a NULL parent) both start a fresh root span.
    #[test]
    fn no_parent_starts_root_span() {
        let exporter = InMemorySpanExporter::default();
        let provider = provider_with_exporter(exporter.clone());
        unsafe {
            let tracer = otel_tracer_provider_get_tracer(provider, sv("scope"), empty(), empty());

            // Explicit options with a NULL parent behaves like no parent.
            let opts = OtelSpanStartOptions {
                kind: OtelSpanKind::Internal as u32,
                parent: ptr::null(),
            };
            let root = otel_tracer_start_span(tracer, sv("root"), &opts);
            assert!(!root.is_null());
            assert_eq!(otel_span_end(root), OtelStatus::Ok);

            let spans = exporter.get_finished_spans().expect("exported spans");
            let root_data = spans
                .iter()
                .find(|s| s.name == "root")
                .expect("root exported");
            assert_ne!(root_data.span_context.trace_id(), TraceId::INVALID);
            assert_eq!(root_data.parent_span_id, SpanId::INVALID);
            assert!(!root_data.parent_span_is_remote);

            otel_span_destroy(root);
            otel_tracer_destroy(tracer);
            otel_tracer_provider_destroy(provider);
        }
    }

    /// Two children of the same parent are siblings: same trace, same parent span id,
    /// distinct span ids.
    #[test]
    fn siblings_share_parent_and_trace() {
        let exporter = InMemorySpanExporter::default();
        let provider = provider_with_exporter(exporter.clone());
        unsafe {
            let tracer = otel_tracer_provider_get_tracer(provider, sv("scope"), empty(), empty());
            let parent = otel_tracer_start_span(tracer, sv("p"), ptr::null());
            let parent_ctx = (*parent).span.span_context().clone();

            let opts = OtelSpanStartOptions {
                kind: OtelSpanKind::Internal as u32,
                parent: parent as *const OtelSpan,
            };
            let a = otel_tracer_start_span(tracer, sv("a"), &opts);
            let b = otel_tracer_start_span(tracer, sv("b"), &opts);
            assert_eq!(otel_span_end(a), OtelStatus::Ok);
            assert_eq!(otel_span_end(b), OtelStatus::Ok);
            assert_eq!(otel_span_end(parent), OtelStatus::Ok);

            let spans = exporter.get_finished_spans().expect("exported spans");
            let a_data = spans.iter().find(|s| s.name == "a").expect("a");
            let b_data = spans.iter().find(|s| s.name == "b").expect("b");

            assert_eq!(a_data.parent_span_id, parent_ctx.span_id());
            assert_eq!(b_data.parent_span_id, parent_ctx.span_id());
            assert_eq!(a_data.span_context.trace_id(), parent_ctx.trace_id());
            assert_eq!(b_data.span_context.trace_id(), parent_ctx.trace_id());
            assert_ne!(a_data.span_context.span_id(), b_data.span_context.span_id());
            assert!(!a_data.parent_span_is_remote);
            assert!(!b_data.parent_span_is_remote);

            otel_span_destroy(a);
            otel_span_destroy(b);
            otel_span_destroy(parent);
            otel_tracer_destroy(tracer);
            otel_tracer_provider_destroy(provider);
        }
    }
}
