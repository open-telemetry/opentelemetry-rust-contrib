//! The SDK's implementation of the internal [`OtelImplVtable`].
//!
//! These `extern "C"` functions are the concrete, `opentelemetry_sdk`-backed behavior
//! behind the API's opaque handles. Their addresses populate a single `'static`
//! [`SDK_VTABLE`]; the API stores a `*const OtelImplVtable` in each handle it creates for
//! an SDK-backed object. Every function is panic-guarded (a Rust panic must never unwind
//! across the C ABI into the API cdylib). No Rust types cross the boundary; contexts are
//! opaque `*mut c_void` that only this crate allocates and frees.

use std::borrow::Cow;
use std::os::raw::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::SystemTime;

use opentelemetry::global::{BoxedSpan, BoxedTracer};
use opentelemetry::trace::{
    Span, SpanContext, SpanKind, Status, TraceContextExt, Tracer, TracerProvider,
};
use opentelemetry::{Context, InstrumentationScope, Key, KeyValue, StringValue, Value};
use opentelemetry_sdk::trace::SdkTracerProvider;

use opentelemetry_c_abi::{
    OtelAttributeType, OtelImplVtable, OtelKeyValue, OtelSpanKind, OtelSpanStatusCode, OtelStatus,
    OtelStringView,
};

use crate::error::{fail, fail_abi};

/// Upper bound on a C-provided event attribute count (protects the up-front `Vec`).
const MAX_EVENT_ATTRIBUTES: usize = 1_048_576;

// ---- Context types (opaque `*mut c_void` on the wire) ----------------------

/// A span context: the boxed `BoxedSpan`.
struct SdkSpan {
    span: BoxedSpan,
}

/// # Safety
/// `ctx` must be a live span context produced by this vtable, used single-threaded.
unsafe fn span_mut<'a>(ctx: *mut c_void) -> &'a mut SdkSpan {
    unsafe { &mut *(ctx as *mut SdkSpan) }
}

// ---- Panic guards ----------------------------------------------------------

fn guard_ptr<F: FnOnce() -> *mut c_void>(f: F) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(std::ptr::null_mut())
}
fn guard_status<F: FnOnce() -> OtelStatus>(f: F) -> OtelStatus {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(OtelStatus::InternalError)
}
fn guard_unit<F: FnOnce()>(f: F) {
    let _ = catch_unwind(AssertUnwindSafe(f));
}

// ---- A minimal local-parent span (preserves local, non-remote parenting) ---

struct LocalParentSpan {
    span_context: SpanContext,
}

impl LocalParentSpan {
    fn new(parent: &SpanContext) -> Self {
        LocalParentSpan {
            span_context: SpanContext::new(
                parent.trace_id(),
                parent.span_id(),
                parent.trace_flags(),
                false, // force is_remote = false: this is an in-process parent
                parent.trace_state().clone(),
            ),
        }
    }
}

impl Span for LocalParentSpan {
    fn add_event_with_timestamp<T>(&mut self, _n: T, _t: SystemTime, _a: Vec<KeyValue>)
    where
        T: Into<Cow<'static, str>>,
    {
    }
    fn span_context(&self) -> &SpanContext {
        &self.span_context
    }
    fn is_recording(&self) -> bool {
        false
    }
    fn set_attribute(&mut self, _a: KeyValue) {}
    fn set_status(&mut self, _s: Status) {}
    fn update_name<T>(&mut self, _n: T)
    where
        T: Into<Cow<'static, str>>,
    {
    }
    fn add_link(&mut self, _sc: SpanContext, _a: Vec<KeyValue>) {}
    fn end_with_timestamp(&mut self, _t: SystemTime) {}
}

// ---- Attribute conversion --------------------------------------------------

/// Convert a borrowed C attribute into an owned [`KeyValue`] (lossy keys/strings).
///
/// # Safety
/// String views inside `kv` must be valid.
pub(crate) unsafe fn to_key_value(kv: &OtelKeyValue) -> Result<KeyValue, OtelStatus> {
    // SAFETY: forwarded to the caller's contract.
    let key = unsafe { kv.key.to_string_lossy() }.map_err(fail_abi)?;
    if key.is_empty() {
        return Err(fail(
            OtelStatus::InvalidArgument,
            "attribute key must not be empty",
        ));
    }
    let value_type = OtelAttributeType::from_u32(kv.value_type).ok_or_else(|| {
        fail(
            OtelStatus::InvalidArgument,
            "attribute value_type is not a valid OtelAttributeType tag",
        )
    })?;
    let value: Value = match value_type {
        OtelAttributeType::String => {
            // SAFETY: tag guarantees the string member is active.
            let s = unsafe { kv.value.string_value.to_string_lossy() }.map_err(fail_abi)?;
            Value::String(StringValue::from(s))
        }
        // SAFETY: tag guarantees the respective member is active.
        OtelAttributeType::Bool => Value::Bool(unsafe { kv.value.bool_value } != 0),
        OtelAttributeType::Int64 => Value::I64(unsafe { kv.value.int64_value }),
        OtelAttributeType::Double => Value::F64(unsafe { kv.value.double_value }),
    };
    Ok(KeyValue::new(Key::from(key), value))
}

/// Collect a borrowed C attribute array into owned [`KeyValue`]s, with bounds/overflow
/// guards and fallible reservation.
///
/// # Safety
/// `attributes` must point to `count` valid [`OtelKeyValue`]s, or be NULL when `count == 0`.
unsafe fn collect_key_values(
    attributes: *const OtelKeyValue,
    count: usize,
) -> Result<Vec<KeyValue>, OtelStatus> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if attributes.is_null() {
        return Err(fail(
            OtelStatus::InvalidArgument,
            "attribute array is NULL with non-zero count",
        ));
    }
    if count > MAX_EVENT_ATTRIBUTES {
        return Err(fail(
            OtelStatus::InvalidArgument,
            "event attribute count exceeds the maximum supported value",
        ));
    }
    let within_bounds = count
        .checked_mul(std::mem::size_of::<OtelKeyValue>())
        .is_some_and(|b| b <= isize::MAX as usize);
    if !within_bounds {
        return Err(fail(
            OtelStatus::InvalidArgument,
            "event attribute count exceeds the maximum supported size",
        ));
    }
    let mut out: Vec<KeyValue> = Vec::new();
    if out.try_reserve(count).is_err() {
        return Err(fail(
            OtelStatus::InternalError,
            "failed to allocate space for event attributes",
        ));
    }
    // SAFETY: non-NULL, `count` valid elements, total size within isize::MAX.
    let slice = unsafe { std::slice::from_raw_parts(attributes, count) };
    for kv in slice {
        // SAFETY: each element satisfies the OtelKeyValue contract.
        out.push(unsafe { to_key_value(kv) }?);
    }
    Ok(out)
}

// ---- Provider vtable -------------------------------------------------------

extern "C" fn vt_provider_get_tracer(
    ctx: *mut c_void,
    name: OtelStringView,
    version: OtelStringView,
    schema_url: OtelStringView,
) -> *mut c_void {
    guard_ptr(|| {
        if ctx.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: `ctx` is a live provider context produced by this crate.
        let provider = unsafe { &*(ctx as *const SdkTracerProvider) };
        // SAFETY: string views satisfy the ABI contract.
        let name = match unsafe { name.to_string_lossy() } {
            Ok(n) => n,
            Err(e) => {
                fail_abi(e);
                return std::ptr::null_mut();
            }
        };
        let mut scope = InstrumentationScope::builder(name);
        // SAFETY: string views satisfy the ABI contract.
        match unsafe { version.to_string_lossy() } {
            Ok(v) if !v.is_empty() => scope = scope.with_version(v),
            Ok(_) => {}
            Err(e) => {
                fail_abi(e);
                return std::ptr::null_mut();
            }
        }
        match unsafe { schema_url.to_string_lossy() } {
            Ok(s) if !s.is_empty() => scope = scope.with_schema_url(s),
            Ok(_) => {}
            Err(e) => {
                fail_abi(e);
                return std::ptr::null_mut();
            }
        }
        let tracer: BoxedTracer =
            BoxedTracer::new(Box::new(provider.tracer_with_scope(scope.build())));
        Box::into_raw(Box::new(tracer)) as *mut c_void
    })
}

extern "C" fn vt_provider_retain(ctx: *mut c_void) -> *mut c_void {
    guard_ptr(|| {
        if ctx.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: `ctx` is a live provider context (Box<SdkTracerProvider>) produced by this
        // crate; the API only calls this while the reference is guaranteed alive (under its
        // global read lock, or for a handle it owns). Clone the Arc-backed provider into a
        // new owned Box — an independent reference that outlives slot replacement.
        let provider = unsafe { &*(ctx as *const SdkTracerProvider) };
        Box::into_raw(Box::new(provider.clone())) as *mut c_void
    })
}

extern "C" fn vt_provider_free(ctx: *mut c_void) {
    guard_unit(|| {
        if !ctx.is_null() {
            // SAFETY: `ctx` was a Box<SdkTracerProvider> produced by this crate. Dropping it
            // releases exactly one Arc reference; the provider lives while any reference
            // (slot or retained) remains.
            drop(unsafe { Box::from_raw(ctx as *mut SdkTracerProvider) });
        }
    });
}

// ---- Tracer vtable ---------------------------------------------------------

extern "C" fn vt_tracer_start_span(
    ctx: *mut c_void,
    name: OtelStringView,
    kind: u32,
    parent_span_ctx: *mut c_void,
) -> *mut c_void {
    guard_ptr(|| {
        if ctx.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: `ctx` is a live tracer context produced by this crate.
        let tracer = unsafe { &*(ctx as *const BoxedTracer) };
        // SAFETY: string view satisfies the ABI contract.
        let name = match unsafe { name.to_string_lossy() } {
            Ok(n) => n,
            Err(e) => {
                fail_abi(e);
                return std::ptr::null_mut();
            }
        };
        let span_kind: SpanKind =
            match OtelSpanKind::from_u32(kind).unwrap_or(OtelSpanKind::Internal) {
                OtelSpanKind::Internal => SpanKind::Internal,
                OtelSpanKind::Server => SpanKind::Server,
                OtelSpanKind::Client => SpanKind::Client,
                OtelSpanKind::Producer => SpanKind::Producer,
                OtelSpanKind::Consumer => SpanKind::Consumer,
            };

        let builder = tracer.span_builder(name).with_kind(span_kind);
        let span: BoxedSpan = if parent_span_ctx.is_null() {
            tracer.build_with_context(builder, &Context::new())
        } else {
            // SAFETY: the API only passes a parent context produced by THIS vtable.
            let parent = unsafe { &*(parent_span_ctx as *const SdkSpan) };
            let cx = Context::new().with_span(LocalParentSpan::new(parent.span.span_context()));
            tracer.build_with_context(builder, &cx)
        };
        Box::into_raw(Box::new(SdkSpan { span })) as *mut c_void
    })
}

extern "C" fn vt_tracer_free(ctx: *mut c_void) {
    guard_unit(|| {
        if !ctx.is_null() {
            // SAFETY: `ctx` was a Box<BoxedTracer> produced by this crate.
            drop(unsafe { Box::from_raw(ctx as *mut BoxedTracer) });
        }
    });
}

// ---- Span vtable -----------------------------------------------------------

extern "C" fn vt_span_set_string(
    ctx: *mut c_void,
    key: OtelStringView,
    value: OtelStringView,
) -> OtelStatus {
    guard_status(|| {
        // SAFETY: `ctx` live span, single-threaded per contract.
        let span = unsafe { span_mut(ctx) };
        let key = match unsafe { key.to_string_lossy() } {
            Ok(k) if !k.is_empty() => k,
            Ok(_) => {
                return fail(
                    OtelStatus::InvalidArgument,
                    "attribute key must not be empty",
                )
            }
            Err(e) => return fail_abi(e),
        };
        let value = match unsafe { value.to_string_lossy() } {
            Ok(v) => v,
            Err(e) => return fail_abi(e),
        };
        span.span.set_attribute(KeyValue::new(key, value));
        OtelStatus::Ok
    })
}

extern "C" fn vt_span_set_bool(ctx: *mut c_void, key: OtelStringView, value: u32) -> OtelStatus {
    guard_status(|| {
        let span = unsafe { span_mut(ctx) };
        let key = match unsafe { key.to_string_lossy() } {
            Ok(k) if !k.is_empty() => k,
            Ok(_) => {
                return fail(
                    OtelStatus::InvalidArgument,
                    "attribute key must not be empty",
                )
            }
            Err(e) => return fail_abi(e),
        };
        span.span.set_attribute(KeyValue::new(key, value != 0));
        OtelStatus::Ok
    })
}

extern "C" fn vt_span_set_i64(ctx: *mut c_void, key: OtelStringView, value: i64) -> OtelStatus {
    guard_status(|| {
        let span = unsafe { span_mut(ctx) };
        let key = match unsafe { key.to_string_lossy() } {
            Ok(k) if !k.is_empty() => k,
            Ok(_) => {
                return fail(
                    OtelStatus::InvalidArgument,
                    "attribute key must not be empty",
                )
            }
            Err(e) => return fail_abi(e),
        };
        span.span.set_attribute(KeyValue::new(key, value));
        OtelStatus::Ok
    })
}

extern "C" fn vt_span_set_f64(ctx: *mut c_void, key: OtelStringView, value: f64) -> OtelStatus {
    guard_status(|| {
        let span = unsafe { span_mut(ctx) };
        let key = match unsafe { key.to_string_lossy() } {
            Ok(k) if !k.is_empty() => k,
            Ok(_) => {
                return fail(
                    OtelStatus::InvalidArgument,
                    "attribute key must not be empty",
                )
            }
            Err(e) => return fail_abi(e),
        };
        span.span.set_attribute(KeyValue::new(key, value));
        OtelStatus::Ok
    })
}

extern "C" fn vt_span_add_event(
    ctx: *mut c_void,
    name: OtelStringView,
    attributes: *const OtelKeyValue,
    attribute_count: usize,
) -> OtelStatus {
    guard_status(|| {
        let span = unsafe { span_mut(ctx) };
        let name = match unsafe { name.to_string_lossy() } {
            Ok(n) => n,
            Err(e) => return fail_abi(e),
        };
        let attrs = match unsafe { collect_key_values(attributes, attribute_count) } {
            Ok(a) => a,
            Err(status) => return status,
        };
        span.span.add_event(name, attrs);
        OtelStatus::Ok
    })
}

extern "C" fn vt_span_set_status(
    ctx: *mut c_void,
    code: u32,
    description: OtelStringView,
) -> OtelStatus {
    guard_status(|| {
        let span = unsafe { span_mut(ctx) };
        let code = match OtelSpanStatusCode::from_u32(code) {
            Some(c) => c,
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
                let desc = match unsafe { description.to_string_lossy() } {
                    Ok(d) => d,
                    Err(e) => return fail_abi(e),
                };
                Status::error(desc)
            }
        };
        span.span.set_status(status);
        OtelStatus::Ok
    })
}

extern "C" fn vt_span_update_name(ctx: *mut c_void, name: OtelStringView) -> OtelStatus {
    guard_status(|| {
        let span = unsafe { span_mut(ctx) };
        let name = match unsafe { name.to_string_lossy() } {
            Ok(n) => n,
            Err(e) => return fail_abi(e),
        };
        span.span.update_name(name);
        OtelStatus::Ok
    })
}

extern "C" fn vt_span_end(ctx: *mut c_void) {
    guard_unit(|| {
        // SAFETY: `ctx` live span, single-threaded per contract.
        let span = unsafe { span_mut(ctx) };
        span.span.end();
    });
}

extern "C" fn vt_span_free(ctx: *mut c_void) {
    guard_unit(|| {
        if !ctx.is_null() {
            // SAFETY: `ctx` was a Box<SdkSpan> produced by this crate. The API ends the span
            // (via vt_span_end) before freeing; dropping the BoxedSpan also ends it if it was
            // not already ended (the SDK span tracks its ended state, so this never
            // double-ends). This matches the OtelImplVtable::span_free ownership contract.
            drop(unsafe { Box::from_raw(ctx as *mut SdkSpan) });
        }
    });
}

/// The single `'static` implementation vtable installed into the API global slot.
pub(crate) static SDK_VTABLE: OtelImplVtable = OtelImplVtable {
    provider_get_tracer: vt_provider_get_tracer,
    provider_retain: vt_provider_retain,
    provider_free: vt_provider_free,
    tracer_start_span: vt_tracer_start_span,
    tracer_free: vt_tracer_free,
    span_set_string: vt_span_set_string,
    span_set_bool: vt_span_set_bool,
    span_set_i64: vt_span_set_i64,
    span_set_f64: vt_span_set_f64,
    span_add_event: vt_span_add_event,
    span_set_status: vt_span_set_status,
    span_update_name: vt_span_update_name,
    span_end: vt_span_end,
    span_free: vt_span_free,
};

/// Pointer to the SDK vtable (installed via the API registration ABI).
pub(crate) fn vtable_ptr() -> *const OtelImplVtable {
    &SDK_VTABLE
}

/// Box a cloned SDK provider into an opaque provider context for the API slot/handle.
pub(crate) fn provider_ctx(provider: SdkTracerProvider) -> *mut c_void {
    Box::into_raw(Box::new(provider)) as *mut c_void
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::SpanId;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SimpleSpanProcessor};
    use std::os::raw::c_char;

    fn sv(s: &str) -> OtelStringView {
        OtelStringView {
            ptr: s.as_ptr().cast::<c_char>(),
            len: s.len(),
        }
    }
    fn empty() -> OtelStringView {
        OtelStringView {
            ptr: std::ptr::null(),
            len: 0,
        }
    }

    /// The SDK vtable must reproduce local parent/child semantics and attribute handling
    /// (this is the same behavior exercised end-to-end by the cross-artifact C test, but
    /// verified here directly against an in-memory exporter).
    #[test]
    fn vtable_parent_child_and_attributes() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_span_processor(SimpleSpanProcessor::new(exporter.clone()))
            .build();
        let vt = &SDK_VTABLE;
        let pctx = provider_ctx(provider);
        let tctx = (vt.provider_get_tracer)(pctx, sv("scope"), sv("1.0"), empty());
        assert!(!tctx.is_null());

        let parent = (vt.tracer_start_span)(tctx, sv("parent"), 0, std::ptr::null_mut());
        assert!(!parent.is_null());
        assert_eq!(
            (vt.span_set_string)(parent, sv("component"), sv("demo")),
            OtelStatus::Ok
        );
        assert_eq!((vt.span_set_i64)(parent, sv("n"), 7), OtelStatus::Ok);
        // empty key rejected
        assert_eq!(
            (vt.span_set_bool)(parent, sv(""), 1),
            OtelStatus::InvalidArgument
        );

        // child linked to parent (kind=2 client)
        let child = (vt.tracer_start_span)(tctx, sv("child"), 2, parent);
        assert!(!child.is_null());
        (vt.span_end)(child);
        (vt.span_end)(parent);

        let spans = exporter.get_finished_spans().unwrap();
        let c = spans.iter().find(|s| s.name == "child").expect("child");
        let p = spans.iter().find(|s| s.name == "parent").expect("parent");
        assert_eq!(c.span_context.trace_id(), p.span_context.trace_id());
        assert_eq!(c.parent_span_id, p.span_context.span_id());
        assert!(!c.parent_span_is_remote);
        assert_eq!(p.parent_span_id, SpanId::INVALID);

        (vt.span_free)(child);
        (vt.span_free)(parent);
        (vt.tracer_free)(tctx);
        (vt.provider_free)(pctx);
    }

    #[test]
    fn child_can_end_before_parent() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_span_processor(SimpleSpanProcessor::new(exporter.clone()))
            .build();
        let vt = &SDK_VTABLE;
        let pctx = provider_ctx(provider);
        let tctx = (vt.provider_get_tracer)(pctx, sv("s"), empty(), empty());
        let parent = (vt.tracer_start_span)(tctx, sv("p"), 0, std::ptr::null_mut());
        let child = (vt.tracer_start_span)(tctx, sv("c"), 0, parent);
        (vt.span_end)(child); // child ends first
        (vt.span_end)(parent);
        let names: Vec<_> = exporter
            .get_finished_spans()
            .unwrap()
            .into_iter()
            .map(|s| s.name.to_string())
            .collect();
        assert!(names.contains(&"c".to_string()) && names.contains(&"p".to_string()));
        (vt.span_free)(child);
        (vt.span_free)(parent);
        (vt.tracer_free)(tctx);
        (vt.provider_free)(pctx);
    }
}
