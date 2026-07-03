//! Backed-implementation failure propagation (contract for item 2 of the hardening spec).
//!
//! A *backed* provider/tracer (one with a real vtable) that fails — e.g. because a caller
//! passed a malformed `otel_string_view_t` — must surface as **NULL** with the last-error
//! left set, NOT as a success-shaped no-op handle. The genuinely unbacked path (no SDK
//! installed) must still return valid no-op handles.
//!
//! The test installs a minimal vtable whose `provider_get_tracer` / `tracer_start_span`
//! reproduce the SDK's behavior: convert the name string view and, on the abi's
//! validation error, set the last-error and return NULL. This exercises the API's
//! propagation logic directly without depending on the SDK crate.

use std::os::raw::{c_char, c_void};

use opentelemetry_c_abi::{OtelImplVtable, OtelKeyValue, OtelStatus, OtelStringView};

use opentelemetry_c_api::{
    otel_api_provider_new, otel_api_set_last_error, otel_global_tracer_provider,
    otel_last_error_message, otel_span_destroy, otel_span_end, otel_tracer_destroy,
    otel_tracer_provider_destroy, otel_tracer_provider_get_tracer, otel_tracer_start_span,
    OtelSpan, OtelSpanStartOptions,
};

// ---- A minimal backed vtable that validates the name like the real SDK ----

fn set_err(msg: &str) {
    // SAFETY: `msg` is a valid UTF-8 byte range for the duration of the call.
    unsafe { otel_api_set_last_error(msg.as_ptr() as *const c_char, msg.len()) };
}

/// Validate `name` exactly as the SDK does; on a malformed view, set the last-error and
/// return `false` (the caller then returns NULL).
fn name_is_valid(name: OtelStringView) -> bool {
    // SAFETY: forwarded to the abi contract; returns Err on NULL+len or oversized len.
    match unsafe { name.to_string_lossy() } {
        Ok(_) => true,
        Err(_) => {
            set_err("malformed name string view");
            false
        }
    }
}

fn dummy() -> *mut c_void {
    Box::into_raw(Box::new(0u8)) as *mut c_void
}
unsafe fn free_dummy(ctx: *mut c_void) {
    if !ctx.is_null() {
        drop(unsafe { Box::from_raw(ctx as *mut u8) });
    }
}

extern "C" fn vt_provider_get_tracer(
    _c: *mut c_void,
    name: OtelStringView,
    _v: OtelStringView,
    _s: OtelStringView,
) -> *mut c_void {
    if !name_is_valid(name) {
        return std::ptr::null_mut();
    }
    dummy()
}
extern "C" fn vt_provider_retain(_c: *mut c_void) -> *mut c_void {
    dummy()
}
extern "C" fn vt_provider_free(c: *mut c_void) {
    unsafe { free_dummy(c) };
}
extern "C" fn vt_tracer_start_span(
    _c: *mut c_void,
    name: OtelStringView,
    _k: u32,
    _p: *mut c_void,
) -> *mut c_void {
    if !name_is_valid(name) {
        return std::ptr::null_mut();
    }
    dummy()
}
extern "C" fn vt_tracer_free(c: *mut c_void) {
    unsafe { free_dummy(c) };
}
extern "C" fn vt_span_str(_c: *mut c_void, _k: OtelStringView, _v: OtelStringView) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn vt_span_bool(_c: *mut c_void, _k: OtelStringView, _v: u32) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn vt_span_i64(_c: *mut c_void, _k: OtelStringView, _v: i64) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn vt_span_f64(_c: *mut c_void, _k: OtelStringView, _v: f64) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn vt_span_event(
    _c: *mut c_void,
    _n: OtelStringView,
    _a: *const OtelKeyValue,
    _cnt: usize,
) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn vt_span_status(_c: *mut c_void, _code: u32, _d: OtelStringView) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn vt_span_update(_c: *mut c_void, _n: OtelStringView) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn vt_span_end(_c: *mut c_void) {}
extern "C" fn vt_span_free(c: *mut c_void) {
    unsafe { free_dummy(c) };
}

static BACKED_VTABLE: OtelImplVtable = OtelImplVtable {
    provider_get_tracer: vt_provider_get_tracer,
    provider_retain: vt_provider_retain,
    provider_free: vt_provider_free,
    tracer_start_span: vt_tracer_start_span,
    tracer_free: vt_tracer_free,
    span_set_string: vt_span_str,
    span_set_bool: vt_span_bool,
    span_set_i64: vt_span_i64,
    span_set_f64: vt_span_f64,
    span_add_event: vt_span_event,
    span_set_status: vt_span_status,
    span_update_name: vt_span_update,
    span_end: vt_span_end,
    span_free: vt_span_free,
};

fn good(s: &'static str) -> OtelStringView {
    OtelStringView {
        ptr: s.as_ptr() as *const c_char,
        len: s.len(),
    }
}
fn empty() -> OtelStringView {
    OtelStringView {
        ptr: std::ptr::null(),
        len: 0,
    }
}
/// A malformed view: NULL pointer with a non-zero length (rejected by the abi).
fn malformed() -> OtelStringView {
    OtelStringView {
        ptr: std::ptr::null(),
        len: 5,
    }
}
fn last_error_is_set() -> bool {
    !otel_last_error_message().ptr.is_null()
}

fn backed_provider() -> *mut opentelemetry_c_api::OtelTracerProvider {
    // SAFETY: BACKED_VTABLE is 'static; the ctx is an owned dummy Box freed on destroy.
    unsafe { otel_api_provider_new(&BACKED_VTABLE, dummy()) }
}

#[test]
fn backed_provider_get_tracer_malformed_name_returns_null() {
    let provider = backed_provider();
    assert!(!provider.is_null());

    // A malformed name view: the backed vtable fails, so the API must return NULL (not a
    // no-op tracer) and leave the last-error set.
    let tracer =
        unsafe { otel_tracer_provider_get_tracer(provider, malformed(), empty(), empty()) };
    assert!(
        tracer.is_null(),
        "backed get_tracer failure must return NULL"
    );
    assert!(
        last_error_is_set(),
        "last-error must remain set after failure"
    );

    // A well-formed name succeeds (proves the vtable is otherwise functional).
    let ok = unsafe { otel_tracer_provider_get_tracer(provider, good("instr"), empty(), empty()) };
    assert!(
        !ok.is_null(),
        "backed get_tracer with a valid name must succeed"
    );
    unsafe { otel_tracer_destroy(ok) };

    unsafe { otel_tracer_provider_destroy(provider) };
}

#[test]
fn backed_tracer_start_span_malformed_name_returns_null() {
    let provider = backed_provider();
    let tracer =
        unsafe { otel_tracer_provider_get_tracer(provider, good("instr"), empty(), empty()) };
    assert!(!tracer.is_null());

    // A malformed span name: the backed tracer fails, so the API must return NULL.
    let span: *mut OtelSpan =
        unsafe { otel_tracer_start_span(tracer, malformed(), std::ptr::null()) };
    assert!(span.is_null(), "backed start_span failure must return NULL");
    assert!(
        last_error_is_set(),
        "last-error must remain set after failure"
    );

    // A well-formed name succeeds.
    let ok: *mut OtelSpan = unsafe { otel_tracer_start_span(tracer, good("op"), std::ptr::null()) };
    assert!(
        !ok.is_null(),
        "backed start_span with a valid name must succeed"
    );
    unsafe {
        otel_span_end(ok);
        otel_span_destroy(ok);
        otel_tracer_destroy(tracer);
        otel_tracer_provider_destroy(provider);
    }
}

#[test]
fn no_sdk_path_returns_noop_handles() {
    // This test file never installs a global SDK, so the global slot is empty: the global
    // provider must yield valid *no-op* handles, and the no-op path must not set an error.
    let provider = otel_global_tracer_provider();
    assert!(!provider.is_null());

    let tracer =
        unsafe { otel_tracer_provider_get_tracer(provider, good("instr"), empty(), empty()) };
    assert!(
        !tracer.is_null(),
        "unbacked get_tracer must return a no-op tracer"
    );
    assert!(!last_error_is_set(), "the no-op path must not set an error");

    let span: *mut OtelSpan =
        unsafe { otel_tracer_start_span(tracer, good("op"), std::ptr::null()) };
    assert!(
        !span.is_null(),
        "unbacked start_span must return a no-op span"
    );

    let opts = OtelSpanStartOptions {
        kind: 0,
        parent: span,
    };
    let child: *mut OtelSpan = unsafe { otel_tracer_start_span(tracer, good("child"), &opts) };
    assert!(!child.is_null(), "no-op child span must be valid");

    unsafe {
        otel_span_end(child);
        otel_span_destroy(child);
        otel_span_end(span);
        otel_span_destroy(span);
        otel_tracer_destroy(tracer);
        otel_tracer_provider_destroy(provider);
    }
}
