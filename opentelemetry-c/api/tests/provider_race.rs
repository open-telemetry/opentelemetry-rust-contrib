//! Concurrency proof for the API-owned global provider slot lifetime.
//!
//! Reproduces the previously-racy pattern: reader threads repeatedly resolve the global
//! provider (`otel_global_tracer_provider` → `otel_tracer_provider_get_tracer` → start/end/
//! destroy a span) while writer threads repeatedly install/replace SDK providers
//! (`otel_api_register_global_provider`). With the `provider_retain` design, the reader's
//! snapshot is an owned reference that stays alive across a concurrent replacement + free,
//! so there is no use-after-free.
//!
//! A **test-only** vtable tracks retain/free/drop counts and poisons freed provider objects,
//! letting us assert deterministically: no dead provider was ever observed (no UAF), every
//! retain was matched by a free (balanced), and every provider except the one still in the
//! slot was dropped exactly once (no leak, no double-free).

use std::os::raw::{c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::SeqCst};
use std::sync::Arc;
use std::time::{Duration, Instant};

use opentelemetry_c_abi::{OtelImplVtable, OtelStatus, OtelStringView};

use opentelemetry_c_api::{
    otel_api_register_global_provider, otel_global_tracer_provider, otel_span_destroy,
    otel_span_end, otel_span_set_string_attribute, otel_tracer_destroy,
    otel_tracer_provider_destroy, otel_tracer_provider_get_tracer, otel_tracer_start_span,
    OtelSpan,
};

const LIVE_MAGIC: u64 = 0xA11A_A11A_A11A_A11A_u64; // arbitrary "alive" marker

static NEW: AtomicUsize = AtomicUsize::new(0);
static DROPPED: AtomicUsize = AtomicUsize::new(0);
static RETAINS: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);
static SAW_DEAD: AtomicBool = AtomicBool::new(false);

/// The shared provider object. `provider_ctx` is a `Box<Arc<TestInner>>` (one owned Arc
/// reference), mirroring the SDK's `Box<SdkTracerProvider>`.
struct TestInner {
    magic: AtomicU64,
}

impl Drop for TestInner {
    fn drop(&mut self) {
        self.magic.store(0, SeqCst); // poison so any later read detects use-after-free
        DROPPED.fetch_add(1, SeqCst);
    }
}

fn new_provider_ctx() -> *mut c_void {
    NEW.fetch_add(1, SeqCst);
    let arc = Arc::new(TestInner {
        magic: AtomicU64::new(LIVE_MAGIC),
    });
    Box::into_raw(Box::new(arc)) as *mut c_void
}

fn check_alive(ctx: *mut c_void) {
    // SAFETY: ctx is a Box<Arc<TestInner>>; the Arc keeps the inner alive for this call.
    let arc = unsafe { &*(ctx as *const Arc<TestInner>) };
    if arc.magic.load(SeqCst) != LIVE_MAGIC {
        SAW_DEAD.store(true, SeqCst);
    }
}

extern "C" fn t_provider_get_tracer(
    ctx: *mut c_void,
    _n: OtelStringView,
    _v: OtelStringView,
    _s: OtelStringView,
) -> *mut c_void {
    check_alive(ctx);
    // Return a dummy, non-null tracer context.
    Box::into_raw(Box::new(0u8)) as *mut c_void
}
extern "C" fn t_provider_retain(ctx: *mut c_void) -> *mut c_void {
    check_alive(ctx);
    RETAINS.fetch_add(1, SeqCst);
    // SAFETY: ctx is a live Box<Arc<TestInner>>; clone the Arc into a new owned Box.
    let arc = unsafe { &*(ctx as *const Arc<TestInner>) };
    Box::into_raw(Box::new(Arc::clone(arc))) as *mut c_void
}
extern "C" fn t_provider_free(ctx: *mut c_void) {
    FREES.fetch_add(1, SeqCst);
    // SAFETY: ctx is a Box<Arc<TestInner>> produced above; drop one reference.
    drop(unsafe { Box::from_raw(ctx as *mut Arc<TestInner>) });
}
extern "C" fn t_tracer_start_span(
    _c: *mut c_void,
    _n: OtelStringView,
    _k: u32,
    _p: *mut c_void,
) -> *mut c_void {
    Box::into_raw(Box::new(0u8)) as *mut c_void
}
extern "C" fn t_tracer_free(ctx: *mut c_void) {
    if !ctx.is_null() {
        drop(unsafe { Box::from_raw(ctx as *mut u8) });
    }
}
extern "C" fn t_span_str(_c: *mut c_void, _k: OtelStringView, _v: OtelStringView) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn t_span_bool(_c: *mut c_void, _k: OtelStringView, _v: u32) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn t_span_i64(_c: *mut c_void, _k: OtelStringView, _v: i64) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn t_span_f64(_c: *mut c_void, _k: OtelStringView, _v: f64) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn t_span_event(
    _c: *mut c_void,
    _n: OtelStringView,
    _a: *const opentelemetry_c_abi::OtelKeyValue,
    _cnt: usize,
) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn t_span_status(_c: *mut c_void, _code: u32, _d: OtelStringView) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn t_span_update(_c: *mut c_void, _n: OtelStringView) -> OtelStatus {
    OtelStatus::Ok
}
extern "C" fn t_span_end(_c: *mut c_void) {}
extern "C" fn t_span_free(ctx: *mut c_void) {
    if !ctx.is_null() {
        drop(unsafe { Box::from_raw(ctx as *mut u8) });
    }
}

static TEST_VTABLE: OtelImplVtable = OtelImplVtable {
    provider_get_tracer: t_provider_get_tracer,
    provider_retain: t_provider_retain,
    provider_free: t_provider_free,
    tracer_start_span: t_tracer_start_span,
    tracer_free: t_tracer_free,
    span_set_string: t_span_str,
    span_set_bool: t_span_bool,
    span_set_i64: t_span_i64,
    span_set_f64: t_span_f64,
    span_add_event: t_span_event,
    span_set_status: t_span_status,
    span_update_name: t_span_update,
    span_end: t_span_end,
    span_free: t_span_free,
};

fn sv(s: &'static str) -> OtelStringView {
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

#[test]
fn global_provider_lifetime_is_race_free() {
    // Install an initial provider so there is always something in the slot.
    unsafe { otel_api_register_global_provider(&TEST_VTABLE, new_provider_ctx()) };

    let stop = Arc::new(AtomicBool::new(false));
    let deadline = Instant::now() + Duration::from_millis(400);
    let mut threads = Vec::new();

    // Readers: resolve the global provider and use a span, concurrently with replacement.
    for _ in 0..4 {
        let stop = Arc::clone(&stop);
        threads.push(std::thread::spawn(move || {
            while !stop.load(SeqCst) {
                unsafe {
                    let p = otel_global_tracer_provider();
                    let t = otel_tracer_provider_get_tracer(p, sv("instr"), empty(), empty());
                    let s: *mut OtelSpan = otel_tracer_start_span(t, sv("op"), std::ptr::null());
                    let _ = otel_span_set_string_attribute(s, sv("k"), sv("v"));
                    let _ = otel_span_end(s);
                    otel_span_destroy(s);
                    otel_tracer_destroy(t);
                    otel_tracer_provider_destroy(p);
                }
            }
        }));
    }
    // Writers: continuously install/replace providers (each replacement frees the old one).
    for _ in 0..2 {
        let stop = Arc::clone(&stop);
        threads.push(std::thread::spawn(move || {
            while !stop.load(SeqCst) {
                unsafe { otel_api_register_global_provider(&TEST_VTABLE, new_provider_ctx()) };
            }
        }));
    }

    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    stop.store(true, SeqCst);
    for t in threads {
        t.join().unwrap();
    }

    // No reader ever observed a freed (poisoned) provider.
    assert!(
        !SAW_DEAD.load(SeqCst),
        "use-after-free: a reader observed a freed global provider context"
    );
    // Meaningful churn happened.
    let new = NEW.load(SeqCst);
    assert!(new >= 2, "expected provider churn, only {new} created");
    let retains = RETAINS.load(SeqCst);
    let frees = FREES.load(SeqCst);
    // Reference-count balance: total owned references created (one slot reference per
    // `new_provider_ctx` + one per `provider_retain`) minus total freed equals exactly ONE
    // — the reference still held by the slot. No leak, no double-free.
    assert_eq!(
        new + retains,
        frees + 1,
        "provider reference imbalance: NEW({new}) + RETAINS({retains}) != FREES({frees}) + 1"
    );
    // Every provider except the one still in the slot was dropped exactly once.
    let dropped = DROPPED.load(SeqCst);
    assert_eq!(
        dropped,
        new - 1,
        "expected {} providers dropped, got {} (leak or double-free)",
        new - 1,
        dropped
    );
}
