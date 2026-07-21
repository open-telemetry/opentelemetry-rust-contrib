//! Hot-path FFI-overhead benchmarks for the **API-only, no-SDK** path.
//!
//! With no SDK installed the global provider resolves to the no-op default, so these
//! benchmarks measure the pure C API boundary cost — opaque handle allocation/validation,
//! the null-vtable no-op check, and panic-guarded dispatch — with **no** SDK work, OTel
//! object allocation, network, or collector. They guard the "API layer is thin" half of the
//! hot-path performance contract (see `opentelemetry-c/README.md`).
//!
//! Setup (handle acquisition) is kept out of the measured span/attribute loops by caching a
//! tracer and, for attribute setters, a span.
//!
//! Run with: `cargo bench -p opentelemetry-c-api`

use std::hint::black_box;
use std::os::raw::c_char;

use criterion::{criterion_group, criterion_main, Criterion};

use opentelemetry_c_api::{
    otel_global_tracer_provider, otel_span_destroy, otel_span_end, otel_span_set_bool_attribute,
    otel_span_set_double_attribute, otel_span_set_int64_attribute, otel_span_set_string_attribute,
    otel_tracer_destroy, otel_tracer_provider_destroy, otel_tracer_provider_get_tracer,
    otel_tracer_start_span, OtelSpan, OtelStringView, OtelTracer,
};

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

/// Acquire a (no-op) tracer via the global provider, for benchmarks that must not measure
/// tracer acquisition. `provider` is released immediately; the tracer handle stays valid.
fn cached_tracer() -> *mut OtelTracer {
    let provider = otel_global_tracer_provider();
    let tracer =
        unsafe { otel_tracer_provider_get_tracer(provider, sv("bench"), sv("0.1.0"), empty()) };
    unsafe { otel_tracer_provider_destroy(provider) };
    tracer
}

fn bench_api_no_sdk(c: &mut Criterion) {
    let mut g = c.benchmark_group("api_no_sdk");

    g.bench_function("global_provider_acquire", |b| {
        b.iter(|| {
            let p = otel_global_tracer_provider();
            black_box(p);
            unsafe { otel_tracer_provider_destroy(p) };
        });
    });

    g.bench_function("tracer_acquire", |b| {
        let provider = otel_global_tracer_provider();
        b.iter(|| {
            let t = unsafe {
                otel_tracer_provider_get_tracer(provider, sv("bench"), sv("0.1.0"), empty())
            };
            black_box(t);
            unsafe { otel_tracer_destroy(t) };
        });
        unsafe { otel_tracer_provider_destroy(provider) };
    });

    g.bench_function("start_end_span", |b| {
        let tracer = cached_tracer();
        b.iter(|| {
            let s: *mut OtelSpan =
                unsafe { otel_tracer_start_span(tracer, sv("op"), std::ptr::null()) };
            unsafe { otel_span_end(s) };
            unsafe { otel_span_destroy(s) };
        });
        unsafe { otel_tracer_destroy(tracer) };
    });

    g.bench_function("set_string_attribute", |b| {
        let tracer = cached_tracer();
        let span = unsafe { otel_tracer_start_span(tracer, sv("op"), std::ptr::null()) };
        b.iter(|| {
            let st = unsafe { otel_span_set_string_attribute(span, sv("http.method"), sv("GET")) };
            black_box(st);
        });
        unsafe {
            otel_span_end(span);
            otel_span_destroy(span);
            otel_tracer_destroy(tracer);
        }
    });

    g.bench_function("set_scalar_attributes", |b| {
        let tracer = cached_tracer();
        let span = unsafe { otel_tracer_start_span(tracer, sv("op"), std::ptr::null()) };
        b.iter(|| unsafe {
            black_box(otel_span_set_int64_attribute(
                span,
                sv("http.status_code"),
                200,
            ));
            black_box(otel_span_set_bool_attribute(span, sv("cache.hit"), 1));
            black_box(otel_span_set_double_attribute(span, sv("duration.ms"), 1.5));
        });
        unsafe {
            otel_span_end(span);
            otel_span_destroy(span);
            otel_tracer_destroy(tracer);
        }
    });

    g.finish();
}

criterion_group!(benches, bench_api_no_sdk);
criterion_main!(benches);
