//! Global provider **retain failure** must surface as NULL, not a no-op tracer.
//!
//! When an SDK/provider is installed but its `provider_retain` returns NULL,
//! `otel_tracer_provider_get_tracer(otel_global_tracer_provider(), ...)` must return NULL
//! with the last-error set — it must NOT collapse to a success-shaped no-op tracer (which is
//! reserved for the genuinely unbacked "no SDK installed" case, covered in `backed_null.rs`).

use std::os::raw::{c_char, c_void};

use opentelemetry_c_abi::{OtelImplVtable, OtelKeyValue, OtelStatus, OtelStringView};

use opentelemetry_c_api::{
    otel_api_register_global_provider, otel_api_set_last_error, otel_global_tracer_provider,
    otel_last_error_message, otel_tracer_destroy, otel_tracer_provider_destroy,
    otel_tracer_provider_get_tracer,
};

fn dummy() -> *mut c_void {
    Box::into_raw(Box::new(0u8)) as *mut c_void
}
unsafe fn free_dummy(ctx: *mut c_void) {
    if !ctx.is_null() {
        drop(unsafe { Box::from_raw(ctx as *mut u8) });
    }
}

// provider_retain variants -------------------------------------------------

/// Fails to retain and records NOTHING — `retain_global` must supply a default error.
extern "C" fn retain_null_no_error(_c: *mut c_void) -> *mut c_void {
    std::ptr::null_mut()
}
/// Fails to retain but records its OWN error first — `retain_global` must preserve it.
extern "C" fn retain_null_with_error(_c: *mut c_void) -> *mut c_void {
    let msg = b"vtable-set retain error";
    // SAFETY: valid pointer/len for the duration of the call.
    unsafe { otel_api_set_last_error(msg.as_ptr().cast::<c_char>(), msg.len()) };
    std::ptr::null_mut()
}

// If retain ever succeeded (it will not here), this would run; make it observable as a
// distinct non-NULL so a regression that skips retain is caught.
extern "C" fn provider_get_tracer(
    _c: *mut c_void,
    _n: OtelStringView,
    _v: OtelStringView,
    _s: OtelStringView,
) -> *mut c_void {
    dummy()
}
extern "C" fn provider_free(c: *mut c_void) {
    unsafe { free_dummy(c) };
}
extern "C" fn tracer_start_span(
    _c: *mut c_void,
    _n: OtelStringView,
    _k: u32,
    _p: *mut c_void,
) -> *mut c_void {
    dummy()
}
extern "C" fn tracer_free(c: *mut c_void) {
    unsafe { free_dummy(c) };
}
extern "C" fn span_str(_c: *mut c_void, _k: OtelStringView, _v: OtelStringView) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn span_bool(_c: *mut c_void, _k: OtelStringView, _v: u32) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn span_i64(_c: *mut c_void, _k: OtelStringView, _v: i64) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn span_f64(_c: *mut c_void, _k: OtelStringView, _v: f64) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn span_event(
    _c: *mut c_void,
    _n: OtelStringView,
    _a: *const OtelKeyValue,
    _cnt: usize,
) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn span_status(_c: *mut c_void, _code: u32, _d: OtelStringView) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn span_update(_c: *mut c_void, _n: OtelStringView) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn span_end(_c: *mut c_void) {}
extern "C" fn span_free(c: *mut c_void) {
    unsafe { free_dummy(c) };
}

const fn vtable_with(retain: extern "C" fn(*mut c_void) -> *mut c_void) -> OtelImplVtable {
    OtelImplVtable {
        provider_get_tracer,
        provider_retain: retain,
        provider_free,
        tracer_start_span,
        tracer_free,
        span_set_string: span_str,
        span_set_bool: span_bool,
        span_set_i64: span_i64,
        span_set_f64: span_f64,
        span_add_event: span_event,
        span_set_status: span_status,
        span_update_name: span_update,
        span_end,
        span_free,
    }
}

static VTABLE_NO_ERROR: OtelImplVtable = vtable_with(retain_null_no_error);
static VTABLE_WITH_ERROR: OtelImplVtable = vtable_with(retain_null_with_error);

fn good(s: &'static str) -> OtelStringView {
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
fn last_error_bytes() -> Option<Vec<u8>> {
    let v = otel_last_error_message();
    if v.ptr.is_null() {
        return None;
    }
    // SAFETY: the view points at the live thread-local error CString.
    Some(unsafe { std::slice::from_raw_parts(v.ptr.cast::<u8>(), v.len) }.to_vec())
}

/// A single test function so the process-global slot is mutated sequentially (integration
/// tests share one process; parallel global installs would race).
#[test]
fn global_retain_failure_returns_null_with_error() {
    // Install a provider whose retain fails WITHOUT recording an error.
    unsafe { otel_api_register_global_provider(&VTABLE_NO_ERROR, dummy()) };

    let provider = otel_global_tracer_provider();
    assert!(!provider.is_null());
    let tracer =
        unsafe { otel_tracer_provider_get_tracer(provider, good("instr"), empty(), empty()) };
    assert!(
        tracer.is_null(),
        "a global provider whose retain fails must return NULL, not a no-op tracer"
    );
    assert_eq!(
        last_error_bytes().as_deref(),
        Some(&b"global provider retain failed"[..]),
        "retain_global must record a default error when the vtable set none"
    );
    unsafe { otel_tracer_provider_destroy(provider) };

    // Install a provider whose retain fails but records ITS OWN error; it must be preserved.
    unsafe { otel_api_register_global_provider(&VTABLE_WITH_ERROR, dummy()) };
    let provider = otel_global_tracer_provider();
    let tracer =
        unsafe { otel_tracer_provider_get_tracer(provider, good("instr"), empty(), empty()) };
    assert!(tracer.is_null(), "retain failure must return NULL");
    assert_eq!(
        last_error_bytes().as_deref(),
        Some(&b"vtable-set retain error"[..]),
        "a vtable-recorded retain error must be preserved, not overwritten"
    );

    // If a regression made this return a (no-op) tracer, ensure we still clean up.
    if !tracer.is_null() {
        unsafe { otel_tracer_destroy(tracer) };
    }
    unsafe { otel_tracer_provider_destroy(provider) };
}
