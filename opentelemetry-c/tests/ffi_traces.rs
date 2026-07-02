//! Integration tests that drive the `#[no_mangle] extern "C"` entry points directly,
//! exactly as a C caller would. These validate the handle lifecycle, null/invalid
//! input handling, panic-free error paths, and idempotent shutdown/end semantics.

use std::os::raw::c_char;
use std::ptr;

use opentelemetry_c::*;

/// Build a length-delimited view over a Rust `&str` (borrowed for the call).
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

/// Build an SDK pointed at a fast-failing local endpoint with short timeouts so the
/// exporter never blocks the test (connection is refused immediately).
fn build_test_sdk() -> *mut OtelSdk {
    unsafe {
        let builder = otel_sdk_builder_new();
        assert!(!builder.is_null());
        assert_eq!(
            otel_sdk_builder_set_service_name(builder, sv("ffi-test")),
            OtelStatus::Ok
        );
        assert_eq!(
            otel_sdk_builder_set_otlp_endpoint(builder, sv("http://127.0.0.1:9/v1/traces")),
            OtelStatus::Ok
        );
        assert_eq!(
            otel_sdk_builder_set_otlp_timeout_millis(builder, 200),
            OtelStatus::Ok
        );
        assert_eq!(
            otel_sdk_builder_set_batch_scheduled_delay_millis(builder, 100),
            OtelStatus::Ok
        );
        let mut sdk: *mut OtelSdk = ptr::null_mut();
        assert_eq!(otel_sdk_build(builder, &mut sdk), OtelStatus::Ok);
        assert!(!sdk.is_null());
        otel_sdk_builder_destroy(builder);
        sdk
    }
}

#[test]
fn version_is_reported() {
    let v = otel_version_string();
    assert!(!v.ptr.is_null());
    assert!(v.len > 0);
    // Matches the crate version components.
    assert_eq!(otel_version_major(), 0);
    // Just ensure these do not panic and are internally consistent.
    let _ = otel_version_minor();
    let _ = otel_version_patch();
}

#[test]
fn null_inputs_are_handled_gracefully() {
    unsafe {
        // Destroys are no-ops on NULL.
        otel_sdk_builder_destroy(ptr::null_mut());
        otel_sdk_destroy(ptr::null_mut());
        otel_tracer_provider_destroy(ptr::null_mut());
        otel_tracer_destroy(ptr::null_mut());
        otel_span_destroy(ptr::null_mut());

        // Getters return NULL on invalid handles.
        assert!(otel_sdk_get_tracer_provider(ptr::null()).is_null());
        assert!(otel_tracer_provider_get_tracer(ptr::null(), sv("t"), empty(), empty()).is_null());
        assert!(otel_tracer_start_span(ptr::null(), sv("s"), ptr::null()).is_null());

        // Status-returning functions reject NULL handles.
        assert_eq!(
            otel_span_set_string_attribute(ptr::null_mut(), sv("k"), sv("v")),
            OtelStatus::InvalidArgument
        );
        assert_eq!(otel_span_end(ptr::null_mut()), OtelStatus::InvalidArgument);
        assert_eq!(
            otel_sdk_shutdown(ptr::null_mut(), 0),
            OtelStatus::InvalidArgument
        );

        // build with NULL out pointer.
        let builder = otel_sdk_builder_new();
        assert_eq!(
            otel_sdk_build(builder, ptr::null_mut()),
            OtelStatus::InvalidArgument
        );
        // build with NULL builder writes NULL and errors.
        let mut sdk: *mut OtelSdk = ptr::null_mut();
        assert_eq!(
            otel_sdk_build(ptr::null(), &mut sdk),
            OtelStatus::InvalidArgument
        );
        assert!(sdk.is_null());
        otel_sdk_builder_destroy(builder);
    }
}

#[test]
fn invalid_utf8_endpoint_is_rejected() {
    unsafe {
        let builder = otel_sdk_builder_new();
        let bad = [0xffu8, 0xfe];
        let view = OtelStringView {
            ptr: bad.as_ptr() as *const c_char,
            len: bad.len(),
        };
        assert_eq!(
            otel_sdk_builder_set_otlp_endpoint(builder, view),
            OtelStatus::InvalidUtf8
        );
        otel_sdk_builder_destroy(builder);
    }
}

#[test]
fn empty_attribute_key_is_rejected() {
    let sdk = build_test_sdk();
    unsafe {
        let provider = otel_sdk_get_tracer_provider(sdk);
        let tracer = otel_tracer_provider_get_tracer(provider, sv("scope"), empty(), empty());
        let span = otel_tracer_start_span(tracer, sv("op"), ptr::null());
        assert!(!span.is_null());
        assert_eq!(
            otel_span_set_string_attribute(span, sv(""), sv("v")),
            OtelStatus::InvalidArgument
        );
        otel_span_end(span);
        otel_span_destroy(span);
        otel_tracer_destroy(tracer);
        otel_tracer_provider_destroy(provider);
        otel_sdk_shutdown(sdk, 500);
        otel_sdk_destroy(sdk);
    }
}

#[test]
fn full_span_lifecycle_via_sdk_provider() {
    let sdk = build_test_sdk();
    unsafe {
        let provider = otel_sdk_get_tracer_provider(sdk);
        assert!(!provider.is_null());
        let tracer =
            otel_tracer_provider_get_tracer(provider, sv("my.scope"), sv("1.2.3"), empty());
        assert!(!tracer.is_null());

        let mut opts = OtelSpanStartOptions {
            kind: OtelSpanKind::Server as u32,
            parent: ptr::null(),
        };
        let parent = otel_tracer_start_span(tracer, sv("parent"), &opts);
        assert!(!parent.is_null());

        assert_eq!(
            otel_span_set_string_attribute(parent, sv("str"), sv("value")),
            OtelStatus::Ok
        );
        assert_eq!(
            otel_span_set_int64_attribute(parent, sv("int"), 42),
            OtelStatus::Ok
        );
        assert_eq!(
            otel_span_set_bool_attribute(parent, sv("bool"), 1),
            OtelStatus::Ok
        );
        assert_eq!(
            otel_span_set_double_attribute(parent, sv("dbl"), 2.71),
            OtelStatus::Ok
        );

        let event_attrs = [OtelKeyValue {
            key: sv("k"),
            value_type: OtelAttributeType::Int64 as u32,
            value: OtelAttributeValue { int64_value: 1 },
        }];
        assert_eq!(
            otel_span_add_event(parent, sv("evt"), event_attrs.as_ptr(), event_attrs.len()),
            OtelStatus::Ok
        );
        // Event with no attributes / NULL array.
        assert_eq!(
            otel_span_add_event(parent, sv("evt2"), ptr::null(), 0),
            OtelStatus::Ok
        );

        assert_eq!(
            otel_span_update_name(parent, sv("parent-renamed")),
            OtelStatus::Ok
        );
        assert_eq!(
            otel_span_set_status(parent, OtelSpanStatusCode::Ok as u32, empty()),
            OtelStatus::Ok
        );

        // Child span linked to the parent.
        opts.kind = OtelSpanKind::Client as u32;
        opts.parent = parent;
        let child = otel_tracer_start_span(tracer, sv("child"), &opts);
        assert!(!child.is_null());
        assert_eq!(
            otel_span_set_status(child, OtelSpanStatusCode::Error as u32, sv("boom")),
            OtelStatus::Ok
        );
        assert_eq!(otel_span_end(child), OtelStatus::Ok);
        otel_span_destroy(child);

        assert_eq!(otel_span_end(parent), OtelStatus::Ok);
        otel_span_destroy(parent);

        otel_tracer_destroy(tracer);
        otel_tracer_provider_destroy(provider);

        // Flush may fail to export (no collector) but must never crash.
        let _ = otel_sdk_force_flush(sdk, 500);
        assert_eq!(otel_sdk_shutdown(sdk, 1000), OtelStatus::Ok);
        otel_sdk_destroy(sdk);
    }
}

#[test]
fn ending_a_span_twice_is_ok() {
    let sdk = build_test_sdk();
    unsafe {
        let provider = otel_sdk_get_tracer_provider(sdk);
        let tracer = otel_tracer_provider_get_tracer(provider, sv("s"), empty(), empty());
        let span = otel_tracer_start_span(tracer, sv("op"), ptr::null());
        assert_eq!(otel_span_end(span), OtelStatus::Ok);
        assert_eq!(otel_span_end(span), OtelStatus::Ok);
        // Setting an attribute after end is a graceful no-op (not a crash).
        assert_eq!(
            otel_span_set_int64_attribute(span, sv("late"), 1),
            OtelStatus::Ok
        );
        otel_span_destroy(span);
        otel_tracer_destroy(tracer);
        otel_tracer_provider_destroy(provider);
        otel_sdk_shutdown(sdk, 500);
        otel_sdk_destroy(sdk);
    }
}

#[test]
fn double_shutdown_and_post_shutdown_calls_are_graceful() {
    let sdk = build_test_sdk();
    unsafe {
        assert_eq!(otel_sdk_shutdown(sdk, 1000), OtelStatus::Ok);
        // Second shutdown is reported but harmless.
        assert_eq!(otel_sdk_shutdown(sdk, 1000), OtelStatus::AlreadyShutdown);
        // Flushing / installing after shutdown is rejected without crashing.
        assert_eq!(otel_sdk_force_flush(sdk, 100), OtelStatus::AlreadyShutdown);
        assert_eq!(otel_sdk_set_as_global(sdk), OtelStatus::AlreadyShutdown);
        otel_sdk_destroy(sdk);
    }
}

#[test]
fn destroy_without_explicit_shutdown_does_not_crash() {
    // The SDK Drop path performs a best-effort shutdown.
    let sdk = build_test_sdk();
    unsafe {
        let provider = otel_sdk_get_tracer_provider(sdk);
        let tracer = otel_tracer_provider_get_tracer(provider, sv("s"), empty(), empty());
        let span = otel_tracer_start_span(tracer, sv("op"), ptr::null());
        otel_span_end(span);
        otel_span_destroy(span);
        otel_tracer_destroy(tracer);
        otel_tracer_provider_destroy(provider);
        otel_sdk_destroy(sdk); // no explicit shutdown
    }
}

#[test]
fn global_provider_install_and_use() {
    let sdk = build_test_sdk();
    unsafe {
        assert_eq!(otel_sdk_set_as_global(sdk), OtelStatus::Ok);
        let provider = otel_global_tracer_provider();
        assert!(!provider.is_null());
        let tracer =
            otel_tracer_provider_get_tracer(provider, sv("global.scope"), empty(), empty());
        assert!(!tracer.is_null());
        let span = otel_tracer_start_span(tracer, sv("global-op"), ptr::null());
        assert!(!span.is_null());
        assert_eq!(
            otel_span_set_string_attribute(span, sv("k"), sv("v")),
            OtelStatus::Ok
        );
        assert_eq!(otel_span_end(span), OtelStatus::Ok);
        otel_span_destroy(span);
        otel_tracer_destroy(tracer);
        otel_tracer_provider_destroy(provider);
        otel_sdk_shutdown(sdk, 1000);
        otel_sdk_destroy(sdk);
    }
}

#[test]
fn out_of_range_discriminants_degrade_safely() {
    // A C caller passing garbage enum/tag values must be handled deliberately, never
    // triggering undefined behavior (e.g. a type-confused union read).
    let sdk = build_test_sdk();
    unsafe {
        let provider = otel_sdk_get_tracer_provider(sdk);
        let tracer = otel_tracer_provider_get_tracer(provider, sv("s"), empty(), empty());

        // Unknown span kind degrades to Internal; the span is still created.
        let opts = OtelSpanStartOptions {
            kind: 9999,
            parent: ptr::null(),
        };
        let span = otel_tracer_start_span(tracer, sv("op"), &opts);
        assert!(!span.is_null());

        // Unknown status code is rejected rather than materializing an invalid enum.
        assert_eq!(
            otel_span_set_status(span, 9999, empty()),
            OtelStatus::InvalidArgument
        );

        // Unknown attribute tag is rejected before the union is read.
        let bad_attr = OtelKeyValue {
            key: sv("k"),
            value_type: 9999,
            value: OtelAttributeValue { int64_value: 0 },
        };
        assert_eq!(
            otel_span_set_attribute(span, bad_attr),
            OtelStatus::InvalidArgument
        );

        // Any non-zero boolean is accepted as true; zero as false.
        assert_eq!(
            otel_span_set_bool_attribute(span, sv("b1"), 255),
            OtelStatus::Ok
        );
        assert_eq!(
            otel_span_set_bool_attribute(span, sv("b2"), 0),
            OtelStatus::Ok
        );

        otel_span_end(span);
        otel_span_destroy(span);
        otel_tracer_destroy(tracer);
        otel_tracer_provider_destroy(provider);
        otel_sdk_shutdown(sdk, 500);
        otel_sdk_destroy(sdk);
    }
}

#[test]
fn concurrent_force_flush_and_shutdown_are_sound() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;

    // A raw handle is not `Send`; wrap it so worker threads can share one pointer, exactly
    // as multiple C threads would operate on a single otel_sdk_t. This is sound because
    // every operation used here takes a shared `&OtelSdk` internally (no `&mut` aliasing).
    #[derive(Clone, Copy)]
    struct SdkPtr(*mut OtelSdk);
    unsafe impl Send for SdkPtr {}
    unsafe impl Sync for SdkPtr {}
    impl SdkPtr {
        // Taking `self` makes closures capture the whole `SdkPtr` (which is `Send`) rather
        // than the bare `*mut` field (which is not) under 2021 disjoint captures.
        fn get(self) -> *mut OtelSdk {
            self.0
        }
    }

    let sdk = build_test_sdk();
    let handle = SdkPtr(sdk);
    // Counts the single call that actually performs the shutdown (i.e. does not observe
    // an already-shut-down SDK). The shutdown-once guarantee means this must be exactly 1.
    let real_shutdowns = Arc::new(AtomicUsize::new(0));

    let mut threads = Vec::new();

    // Several concurrent timed force flushes. Any status is acceptable (Ok / Timeout /
    // AlreadyShutdown); the requirement is only that they do not crash or corrupt state.
    for _ in 0..4 {
        let h = handle;
        threads.push(thread::spawn(move || {
            let _ = unsafe { otel_sdk_force_flush(h.get(), 50) };
        }));
    }

    // Several concurrent shutdowns racing with the flushes.
    for _ in 0..4 {
        let h = handle;
        let reals = Arc::clone(&real_shutdowns);
        threads.push(thread::spawn(move || {
            let status = unsafe { otel_sdk_shutdown(h.get(), 500) };
            if status != OtelStatus::AlreadyShutdown {
                reals.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }

    for t in threads {
        t.join().unwrap();
    }

    assert_eq!(
        real_shutdowns.load(Ordering::SeqCst),
        1,
        "the underlying shutdown must run exactly once under concurrency"
    );

    // Post-shutdown calls are deterministic. Destruction happens only after every worker
    // has joined, honoring the "destroy must not race" contract.
    unsafe {
        assert_eq!(otel_sdk_shutdown(sdk, 100), OtelStatus::AlreadyShutdown);
        assert_eq!(otel_sdk_force_flush(sdk, 100), OtelStatus::AlreadyShutdown);
        otel_sdk_destroy(sdk);
    }
}

#[test]
fn repeated_timed_force_flush_is_bounded() {
    // Sequential timed flushes must each return a well-defined status and must not
    // accumulate unbounded helper threads or corrupt state.
    let sdk = build_test_sdk();
    unsafe {
        for _ in 0..5 {
            let status = otel_sdk_force_flush(sdk, 200);
            assert!(
                matches!(
                    status,
                    OtelStatus::Ok | OtelStatus::Timeout | OtelStatus::ExportFailed
                ),
                "unexpected force flush status: {status:?}"
            );
        }
        assert_eq!(otel_sdk_shutdown(sdk, 500), OtelStatus::Ok);
        otel_sdk_destroy(sdk);
    }
}
