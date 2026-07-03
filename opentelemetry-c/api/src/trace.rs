//! Trace API surface: tracer providers, tracers, and spans as opaque handles.
//!
//! Each handle stores a `*const OtelImplVtable` (NULL = the no-op default) plus an opaque
//! `*mut c_void` context that the SDK cdylib allocated. Every operation dispatches through
//! the vtable when backed, or is a safe no-op when not. No Rust SDK types cross this
//! boundary; the SDK frees its own contexts via the vtable `*_free` entries, and the API
//! frees only the API handle allocations.

use std::os::raw::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use opentelemetry_c_abi::{
    OtelAttributeType, OtelBool, OtelImplVtable, OtelKeyValue, OtelSpanStatusCode, OtelStringView,
};

use crate::error::{clear_last_error, fail, OtelStatus};
use crate::global::{retain_global, GlobalRetain};
use crate::handle::{
    checked_ref, destroy, guard_ptr, guard_status, guard_unit, into_raw, HasMagic,
};

const PROVIDER_MAGIC: u64 = 0x4F54_4C43_5052_4F56; // "OTLCPROV"
const TRACER_MAGIC: u64 = 0x4F54_4C43_5452_4143; // "OTLCTRAC"
const SPAN_MAGIC: u64 = 0x4F54_4C43_5350_414E; // "OTLCSPAN"

/// Backing selector for a provider handle.
pub(crate) enum ProviderInner {
    /// Resolve the process-global slot lazily on each tracer request.
    Global,
    /// A specific SDK-backed provider (owns its context; freed on destroy).
    Backed {
        vtable: *const OtelImplVtable,
        ctx: *mut c_void,
    },
}

/// Opaque tracer-provider handle (`otel_tracer_provider_t`).
pub struct OtelTracerProvider {
    magic: u64,
    inner: ProviderInner,
}

impl OtelTracerProvider {
    pub(crate) fn new(inner: ProviderInner) -> Self {
        OtelTracerProvider {
            magic: PROVIDER_MAGIC,
            inner,
        }
    }
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

/// Opaque tracer handle (`otel_tracer_t`). NULL `vtable` == no-op.
pub struct OtelTracer {
    magic: u64,
    vtable: *const OtelImplVtable,
    ctx: *mut c_void,
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

/// Opaque span handle (`otel_span_t`). NULL `vtable` == no-op.
pub struct OtelSpan {
    magic: u64,
    vtable: *const OtelImplVtable,
    ctx: *mut c_void,
    ended: AtomicBool,
}

impl OtelSpan {
    fn end(&self) {
        if !self.ended.swap(true, Ordering::AcqRel) && !self.vtable.is_null() {
            // SAFETY: `vtable` is a live registered vtable; `ctx` its span context.
            unsafe { ((*self.vtable).span_end)(self.ctx) };
        }
    }

    fn end_and_free_ctx(&self) {
        if self.vtable.is_null() {
            return;
        }
        let ended = self.ended.swap(true, Ordering::AcqRel);
        // SAFETY: `vtable` is live; `ctx` its span context. Free ends it if needed.
        unsafe {
            if !ended {
                ((*self.vtable).span_end)(self.ctx);
            }
            ((*self.vtable).span_free)(self.ctx);
        }
    }
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

// SAFETY: provider and tracer handles are documented as safe to share across threads. Their
// raw pointers reference SDK objects that are `Send + Sync` (Arc-backed provider / BoxedTracer)
// and whose vtable functions take shared access, so concurrent use is sound. (Span handles
// carry a single-thread contract and are intentionally not marked Sync.)
unsafe impl Send for OtelTracerProvider {}
unsafe impl Sync for OtelTracerProvider {}
unsafe impl Send for OtelTracer {}
unsafe impl Sync for OtelTracer {}

// The C contract documents provider/tracer handles as concurrency-safe; assert `Sync` at
// compile time so a future non-`Sync` field breaks the build.
const _: () = {
    fn assert_sync<T: Sync>() {}
    let _ = assert_sync::<OtelTracerProvider>;
    let _ = assert_sync::<OtelTracer>;
};

/// Options for [`otel_tracer_start_span`]. NULL selects `Internal` kind and no parent.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct OtelSpanStartOptions {
    /// Span kind, an [`opentelemetry_c_abi::OtelSpanKind`] value. Unknown => `Internal`.
    pub kind: u32,
    /// Optional parent span; NULL => root span. Borrowed for the call only.
    pub parent: *const OtelSpan,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(std::mem::size_of::<OtelSpanStartOptions>() == 16);
    assert!(std::mem::align_of::<OtelSpanStartOptions>() == 8);
};

fn new_tracer(vtable: *const OtelImplVtable, ctx: *mut c_void) -> *mut OtelTracer {
    into_raw(OtelTracer {
        magic: TRACER_MAGIC,
        vtable,
        ctx,
    })
}

fn new_span(vtable: *const OtelImplVtable, ctx: *mut c_void) -> *mut OtelSpan {
    into_raw(OtelSpan {
        magic: SPAN_MAGIC,
        vtable,
        ctx,
        ended: AtomicBool::new(false),
    })
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

/// Obtain a tracer from a provider.
///
/// - Invalid provider handle: returns NULL.
/// - No SDK installed (unbacked global provider): returns a valid **no-op** tracer.
/// - A backed implementation whose tracer creation fails (e.g. malformed string view or
///   allocation failure): returns NULL with the last-error set — **not** a no-op tracer.
///
/// # Safety
/// `provider` must satisfy the handle contract; the string views must be valid.
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
        // Resolve the backing implementation. For the process-global provider we retain an
        // OWNED reference to its context (under the global read lock) so it cannot be freed
        // by a concurrent replacement while we use it; `owned` marks that we must release it.
        let (vtable, ctx, owned) = match &provider.inner {
            ProviderInner::Global => match retain_global() {
                // No SDK installed: a genuine unbacked provider — a valid no-op tracer.
                GlobalRetain::NoProvider => {
                    return new_tracer(std::ptr::null(), std::ptr::null_mut());
                }
                // A provider IS installed but retaining it failed; `retain_global` left the
                // last-error set. Surface the failure as NULL — NOT a no-op tracer.
                GlobalRetain::RetainFailed => return std::ptr::null_mut(),
                GlobalRetain::Retained { vtable, ctx } => (vtable, ctx, true),
            },
            ProviderInner::Backed { vtable, ctx } => (*vtable, *ctx, false),
        };
        if vtable.is_null() {
            // Defensive: a `Backed` provider is always created with a non-NULL vtable
            // (`otel_api_provider_new` rejects NULL), and `Retained` always carries the live
            // slot vtable. Treat any unexpected NULL as an unbacked no-op rather than
            // dereferencing it.
            return new_tracer(std::ptr::null(), std::ptr::null_mut());
        }
        // SAFETY: `vtable` is a live registered vtable; `ctx` is a valid provider context (an
        // owned retained reference when `owned`, else the Backed handle's own context).
        let tracer_ctx = unsafe { ((*vtable).provider_get_tracer)(ctx, name, version, schema_url) };
        if owned {
            // SAFETY: release the retained global reference exactly once, regardless of the
            // get_tracer result. `provider_free` does not touch the last-error slot.
            unsafe { ((*vtable).provider_free)(ctx) };
        }
        if tracer_ctx.is_null() {
            // A REAL backed/global implementation was asked and failed (malformed view,
            // allocation failure, or a guarded vtable panic); it left the last-error set.
            // Surface the failure as NULL — do NOT clear the error or degrade to a no-op.
            return std::ptr::null_mut();
        }
        new_tracer(vtable, tracer_ctx)
    })
}

/// Destroy a tracer-provider handle (no-op on NULL). Frees a backed provider's context.
///
/// # Safety
/// `provider` must be NULL or a live provider handle, not destroyed concurrently.
#[no_mangle]
pub unsafe extern "C" fn otel_tracer_provider_destroy(provider: *mut OtelTracerProvider) {
    guard_unit(|| {
        if let Some(p) = unsafe { checked_ref::<OtelTracerProvider>(provider) } {
            // Match by reference and copy the raw pointer fields locally — `ProviderInner`
            // is not `Copy`, so this must not move `inner` out of the shared borrow. A
            // `Global` provider owns no context and needs no free.
            if let ProviderInner::Backed { vtable, ctx } = &p.inner {
                let (vtable, ctx) = (*vtable, *ctx);
                if !vtable.is_null() {
                    // SAFETY: `vtable` is live; free the owned provider context exactly once.
                    unsafe { ((*vtable).provider_free)(ctx) };
                }
            }
        }
        // SAFETY: forwarded to the caller's contract.
        unsafe { destroy(provider) };
    });
}

// ---------------------------------------------------------------------------
// Tracer
// ---------------------------------------------------------------------------

/// Start a new span.
///
/// - Invalid tracer handle, malformed `options`, or a non-NULL but invalid `parent`: NULL.
/// - Unbacked (no-op) tracer: returns a valid **no-op** span.
/// - A backed tracer whose span creation fails (e.g. malformed name or allocation failure):
///   returns NULL with the last-error set — **not** a no-op span.
///
/// A parent span produced by a *different* implementation (its vtable differs from this
/// tracer's) is treated as **no parent**, so the new span is a root span. See `trace.h`.
///
/// # Safety
/// `tracer` must satisfy the handle contract; `options` (if non-NULL) must point to a valid
/// [`OtelSpanStartOptions`] whose `parent` is NULL or a live span handle; `name` valid.
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
        // Copy the validated vtable into a local so we don't repeatedly read the handle's raw
        // pointer field after the null check (also clears static-analysis warnings about
        // dereferencing a raw pointer loaded from the handle).
        let vtable = tracer.vtable;
        if vtable.is_null() {
            // Unbacked (no-op) tracer: a valid no-op span, as the spec expects.
            return new_span(std::ptr::null(), std::ptr::null_mut());
        }

        let mut kind: u32 = 0;
        let mut parent_ctx: *mut c_void = std::ptr::null_mut();
        if !options.is_null() {
            // SAFETY: caller guarantees a valid options pointer when non-NULL.
            let options = unsafe { &*options };
            kind = options.kind;
            if !options.parent.is_null() {
                // SAFETY: caller guarantees parent is NULL or a live span handle.
                match unsafe { checked_ref::<OtelSpan>(options.parent) } {
                    // Only pass the parent context if it belongs to the SAME implementation
                    // (same vtable); a parent from a different implementation is treated as
                    // no parent, so the new span is a root span (documented in trace.h).
                    Some(parent) if parent.vtable == vtable => parent_ctx = parent.ctx,
                    Some(_) => {}
                    None => return std::ptr::null_mut(),
                }
            }
        }

        // SAFETY: `vtable` is live; `tracer.ctx` its tracer context.
        let span_ctx = unsafe { ((*vtable).tracer_start_span)(tracer.ctx, name, kind, parent_ctx) };
        if span_ctx.is_null() {
            // A REAL backed tracer was asked and failed (malformed name, allocation failure,
            // or a guarded vtable panic); it left the last-error set. Surface the failure as
            // NULL — do NOT clear the error or degrade to a no-op span.
            return std::ptr::null_mut();
        }
        new_span(vtable, span_ctx)
    })
}

/// Destroy a tracer handle (no-op on NULL). Frees a backed tracer's context.
///
/// # Safety
/// `tracer` must be NULL or a live tracer handle, not destroyed concurrently.
#[no_mangle]
pub unsafe extern "C" fn otel_tracer_destroy(tracer: *mut OtelTracer) {
    guard_unit(|| {
        if let Some(t) = unsafe { checked_ref::<OtelTracer>(tracer) } {
            if !t.vtable.is_null() {
                // SAFETY: `vtable` is live; free the owned tracer context.
                unsafe { ((*t.vtable).tracer_free)(t.ctx) };
            }
        }
        // SAFETY: forwarded to the caller's contract.
        unsafe { destroy(tracer) };
    });
}

// ---------------------------------------------------------------------------
// Span
// ---------------------------------------------------------------------------

/// Run `f` with a validated `&OtelSpan`, dispatching a status. No-op spans (NULL vtable)
/// return `Ok` without calling `f`.
///
/// # Safety
/// `span` must satisfy the handle contract.
unsafe fn dispatch_span<F>(span: *mut OtelSpan, f: F) -> OtelStatus
where
    F: FnOnce(&OtelImplVtable, *mut c_void) -> OtelStatus,
{
    guard_status(|| {
        clear_last_error();
        // SAFETY: forwarded to the caller's contract (single-thread span use).
        match unsafe { checked_ref::<OtelSpan>(span) } {
            Some(s) if s.vtable.is_null() => OtelStatus::Ok,
            // SAFETY: `s.vtable` is a live registered vtable.
            Some(s) => f(unsafe { &*s.vtable }, s.ctx),
            None => OtelStatus::InvalidArgument,
        }
    })
}

/// Set a string attribute on a span.
///
/// # Safety
/// `span` must satisfy the handle contract; the string views must be valid.
#[no_mangle]
pub unsafe extern "C" fn otel_span_set_string_attribute(
    span: *mut OtelSpan,
    key: OtelStringView,
    value: OtelStringView,
) -> OtelStatus {
    unsafe { dispatch_span(span, |vt, ctx| (vt.span_set_string)(ctx, key, value)) }
}

/// Set a boolean attribute (`0` = false, non-zero = true).
///
/// # Safety
/// `span` must satisfy the handle contract; `key` must be valid.
#[no_mangle]
pub unsafe extern "C" fn otel_span_set_bool_attribute(
    span: *mut OtelSpan,
    key: OtelStringView,
    value: OtelBool,
) -> OtelStatus {
    unsafe { dispatch_span(span, |vt, ctx| (vt.span_set_bool)(ctx, key, value)) }
}

/// Set an i64 attribute.
///
/// # Safety
/// `span` must satisfy the handle contract; `key` must be valid.
#[no_mangle]
pub unsafe extern "C" fn otel_span_set_int64_attribute(
    span: *mut OtelSpan,
    key: OtelStringView,
    value: i64,
) -> OtelStatus {
    unsafe { dispatch_span(span, |vt, ctx| (vt.span_set_i64)(ctx, key, value)) }
}

/// Set an f64 attribute.
///
/// # Safety
/// `span` must satisfy the handle contract; `key` must be valid.
#[no_mangle]
pub unsafe extern "C" fn otel_span_set_double_attribute(
    span: *mut OtelSpan,
    key: OtelStringView,
    value: f64,
) -> OtelStatus {
    unsafe { dispatch_span(span, |vt, ctx| (vt.span_set_f64)(ctx, key, value)) }
}

/// Set a typed attribute from an [`OtelKeyValue`], dispatching by tag.
///
/// # Safety
/// `span` must satisfy the handle contract; `attribute` must satisfy its contract.
#[no_mangle]
pub unsafe extern "C" fn otel_span_set_attribute(
    span: *mut OtelSpan,
    attribute: OtelKeyValue,
) -> OtelStatus {
    unsafe {
        dispatch_span(span, |vt, ctx| {
            // SAFETY: the union member matching the validated tag is active. Union access
            // is permitted here without an inner `unsafe` block because the enclosing
            // function is `unsafe`.
            match OtelAttributeType::from_u32(attribute.value_type) {
                Some(OtelAttributeType::String) => {
                    (vt.span_set_string)(ctx, attribute.key, attribute.value.string_value)
                }
                Some(OtelAttributeType::Bool) => {
                    (vt.span_set_bool)(ctx, attribute.key, attribute.value.bool_value)
                }
                Some(OtelAttributeType::Int64) => {
                    (vt.span_set_i64)(ctx, attribute.key, attribute.value.int64_value)
                }
                Some(OtelAttributeType::Double) => {
                    (vt.span_set_f64)(ctx, attribute.key, attribute.value.double_value)
                }
                None => fail(
                    OtelStatus::InvalidArgument,
                    "attribute value_type is not a valid OtelAttributeType tag",
                ),
            }
        })
    }
}

/// Add a timestamped event with optional attributes.
///
/// # Safety
/// `span` must satisfy the handle contract; `name` valid; `attributes` valid for `count`.
#[no_mangle]
pub unsafe extern "C" fn otel_span_add_event(
    span: *mut OtelSpan,
    name: OtelStringView,
    attributes: *const OtelKeyValue,
    attribute_count: usize,
) -> OtelStatus {
    unsafe {
        dispatch_span(span, |vt, ctx| {
            (vt.span_add_event)(ctx, name, attributes, attribute_count)
        })
    }
}

/// Set the span status. `code` outside [`OtelSpanStatusCode`] is rejected.
///
/// # Safety
/// `span` must satisfy the handle contract; `description` valid.
#[no_mangle]
pub unsafe extern "C" fn otel_span_set_status(
    span: *mut OtelSpan,
    code: u32,
    description: OtelStringView,
) -> OtelStatus {
    unsafe {
        dispatch_span(span, |vt, ctx| {
            if OtelSpanStatusCode::from_u32(code).is_none() {
                return fail(
                    OtelStatus::InvalidArgument,
                    "status code is not a valid OtelSpanStatusCode value",
                );
            }
            (vt.span_set_status)(ctx, code, description)
        })
    }
}

/// Rename a span.
///
/// # Safety
/// `span` must satisfy the handle contract; `name` valid.
#[no_mangle]
pub unsafe extern "C" fn otel_span_update_name(
    span: *mut OtelSpan,
    name: OtelStringView,
) -> OtelStatus {
    unsafe { dispatch_span(span, |vt, ctx| (vt.span_update_name)(ctx, name)) }
}

/// End a span (idempotent).
///
/// # Safety
/// `span` must satisfy the handle contract.
#[no_mangle]
pub unsafe extern "C" fn otel_span_end(span: *mut OtelSpan) -> OtelStatus {
    guard_status(|| {
        clear_last_error();
        // SAFETY: forwarded to the caller's contract.
        match unsafe { checked_ref::<OtelSpan>(span) } {
            Some(s) => {
                s.end();
                OtelStatus::Ok
            }
            None => OtelStatus::InvalidArgument,
        }
    })
}

/// Destroy a span handle (no-op on NULL). Best-effort ends it, then frees its context.
///
/// # Safety
/// `span` must be NULL or a live span handle, not used or destroyed concurrently.
#[no_mangle]
pub unsafe extern "C" fn otel_span_destroy(span: *mut OtelSpan) {
    guard_unit(|| {
        // SAFETY: forwarded to the caller's contract.
        if let Some(s) = unsafe { checked_ref::<OtelSpan>(span) } {
            s.end_and_free_ctx();
        }
        // SAFETY: forwarded to the caller's contract.
        unsafe { destroy(span) };
    });
}
