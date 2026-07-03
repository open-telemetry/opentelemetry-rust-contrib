//! Hot-path FFI-overhead benchmarks for the **SDK-backed** path (no collector / no network).
//!
//! These install a *real* Rust OTel SDK trace pipeline (OTLP exporter + batch span processor)
//! as the global provider through the public C SDK API, then drive the public C API span/tracer
//! entrypoints. They measure the true cost of a span/attribute/event through the C boundary plus
//! the Rust SDK's own machinery — the counterpart to the no-op `api_hotpath` bench.
//!
//! No collector is required and nothing is exported over the network: the OTLP exporter targets a
//! closed loopback port, so any batch flush fails fast (connection refused) and is discarded. The
//! batch processor's bounded queue/buffer keeps memory bounded; a very large scheduled delay makes
//! flushing batch-size-driven rather than timer-driven. `force_flush` is never called. This is
//! **not** a network/export throughput benchmark and is not a default regression guard for export.
//!
//! Setup (pipeline build + global install + tracer acquisition) is kept out of the measured loops.
//! Attribute/event setters run on a **fresh** span per iteration via `iter_batched`, so the span's
//! per-span attribute/event limits are never hit and each op measures the real "store" path; the
//! excluded setup/teardown starts and ends+destroys that span.
//!
//! Run with: `cargo bench -p opentelemetry-c-sdk`

use std::hint::black_box;
use std::os::raw::c_char;
use std::ptr;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

// Public C API entrypoints (dev-dep): the real process-global provider slot and span/tracer ops.
use opentelemetry_c_api::{
    otel_global_tracer_provider, otel_span_add_event, otel_span_destroy, otel_span_end,
    otel_span_set_bool_attribute, otel_span_set_double_attribute, otel_span_set_int64_attribute,
    otel_span_set_string_attribute, otel_tracer_destroy, otel_tracer_provider_destroy,
    otel_tracer_provider_get_tracer, otel_tracer_start_span, OtelAttributeType, OtelAttributeValue,
    OtelKeyValue, OtelSpan, OtelStatus, OtelStringView, OtelTracer,
};
// Public C SDK entrypoints (crate under bench): build the pipeline and install it as global.
use opentelemetry_c_sdk::{
    otel_batch_span_processor_builder_build, otel_batch_span_processor_builder_destroy,
    otel_batch_span_processor_builder_new, otel_batch_span_processor_builder_set_exporter,
    otel_batch_span_processor_builder_set_max_export_batch_size,
    otel_batch_span_processor_builder_set_max_queue_size,
    otel_batch_span_processor_builder_set_scheduled_delay_millis,
    otel_otlp_trace_exporter_builder_build, otel_otlp_trace_exporter_builder_destroy,
    otel_otlp_trace_exporter_builder_new, otel_otlp_trace_exporter_builder_set_endpoint,
    otel_sdk_build, otel_sdk_builder_add_span_processor, otel_sdk_builder_destroy,
    otel_sdk_builder_new, otel_sdk_builder_set_service_name, otel_sdk_destroy,
    otel_sdk_set_as_global, otel_sdk_shutdown, OtelSdk, OtelSpanProcessor, OtelTraceExporter,
};

fn sv(s: &str) -> OtelStringView {
    OtelStringView {
        ptr: s.as_ptr().cast::<c_char>(),
        len: s.len(),
    }
}
fn empty() -> OtelStringView {
    OtelStringView {
        ptr: ptr::null(),
        len: 0,
    }
}
fn assert_ok(status: OtelStatus) {
    assert_eq!(status, OtelStatus::Ok, "setup FFI call failed: {status:?}");
}

/// Build a real trace pipeline and install it as the process-global provider. Returns the owned
/// SDK handle (shut down + destroyed at the end of the bench). The OTLP exporter targets a closed
/// loopback port, so nothing is ever exported over the network.
fn install_sdk() -> *mut OtelSdk {
    unsafe {
        // OTLP exporter. `build` does not connect (the client is lazy); only a batch flush would,
        // and it fails fast against the closed port with no collector running.
        let xb = otel_otlp_trace_exporter_builder_new();
        assert_ok(otel_otlp_trace_exporter_builder_set_endpoint(
            xb,
            sv("http://127.0.0.1:1/v1/traces"),
        ));
        let mut exporter: *mut OtelTraceExporter = ptr::null_mut();
        assert_ok(otel_otlp_trace_exporter_builder_build(xb, &mut exporter));
        otel_otlp_trace_exporter_builder_destroy(xb);

        // Batch processor. Bounded queue/batch keep memory bounded; a very large scheduled delay
        // keeps flushing batch-size-driven so no timer-driven export fires during the bench.
        let pb = otel_batch_span_processor_builder_new();
        assert_ok(otel_batch_span_processor_builder_set_exporter(pb, exporter));
        assert_ok(otel_batch_span_processor_builder_set_max_queue_size(
            pb, 8192,
        ));
        assert_ok(otel_batch_span_processor_builder_set_max_export_batch_size(
            pb, 2048,
        ));
        assert_ok(otel_batch_span_processor_builder_set_scheduled_delay_millis(pb, 3_600_000));
        let mut processor: *mut OtelSpanProcessor = ptr::null_mut();
        assert_ok(otel_batch_span_processor_builder_build(pb, &mut processor));
        otel_batch_span_processor_builder_destroy(pb);

        // SDK + global install.
        let sb = otel_sdk_builder_new();
        assert_ok(otel_sdk_builder_set_service_name(sb, sv("otel-c-bench")));
        assert_ok(otel_sdk_builder_add_span_processor(sb, processor));
        let mut sdk: *mut OtelSdk = ptr::null_mut();
        assert_ok(otel_sdk_build(sb, &mut sdk));
        otel_sdk_builder_destroy(sb);

        assert_ok(otel_sdk_set_as_global(sdk));
        sdk
    }
}

/// Acquire a tracer through the installed global provider (SDK-backed). Used by span benches that
/// should not measure tracer acquisition.
fn global_tracer() -> *mut OtelTracer {
    let provider = otel_global_tracer_provider();
    let tracer =
        unsafe { otel_tracer_provider_get_tracer(provider, sv("bench"), sv("0.1.0"), empty()) };
    unsafe { otel_tracer_provider_destroy(provider) };
    assert!(!tracer.is_null(), "SDK-backed tracer acquisition failed");
    tracer
}

/// RAII guard that ends and destroys a span outside the measured region (used as the excluded
/// teardown of `iter_batched`, so the timed routine measures only the setter/event op).
struct SpanGuard(*mut OtelSpan);
impl Drop for SpanGuard {
    fn drop(&mut self) {
        unsafe {
            otel_span_end(self.0);
            otel_span_destroy(self.0);
        }
    }
}

fn bench_sdk_backed(c: &mut Criterion) {
    let sdk = install_sdk();
    let tracer = global_tracer();
    let start = |t: *mut OtelTracer| unsafe { otel_tracer_start_span(t, sv("op"), ptr::null()) };

    let mut g = c.benchmark_group("sdk_backed");

    g.bench_function("tracer_acquire_global", |b| {
        b.iter(|| {
            let provider = otel_global_tracer_provider();
            let t = unsafe {
                otel_tracer_provider_get_tracer(provider, sv("bench"), sv("0.1.0"), empty())
            };
            black_box(t);
            unsafe {
                otel_tracer_destroy(t);
                otel_tracer_provider_destroy(provider);
            }
        });
    });

    g.bench_function("start_end_span", |b| {
        b.iter(|| {
            let s = start(tracer);
            unsafe {
                otel_span_end(s);
                otel_span_destroy(s);
            }
        });
    });

    g.bench_function("set_string_attribute", |b| {
        b.iter_batched(
            || SpanGuard(start(tracer)),
            |guard| {
                let st = unsafe {
                    otel_span_set_string_attribute(guard.0, sv("http.method"), sv("GET"))
                };
                black_box(st);
                guard
            },
            BatchSize::SmallInput,
        );
    });

    g.bench_function("set_scalar_attributes", |b| {
        b.iter_batched(
            || SpanGuard(start(tracer)),
            |guard| {
                unsafe {
                    black_box(otel_span_set_int64_attribute(
                        guard.0,
                        sv("http.status_code"),
                        200,
                    ));
                    black_box(otel_span_set_bool_attribute(guard.0, sv("cache.hit"), 1));
                    black_box(otel_span_set_double_attribute(
                        guard.0,
                        sv("duration.ms"),
                        1.5,
                    ));
                }
                guard
            },
            BatchSize::SmallInput,
        );
    });

    g.bench_function("add_event_bounded_attrs", |b| {
        // A fixed, small ("bounded") set of event attributes built once outside the loop.
        let attrs = [
            OtelKeyValue {
                key: sv("http.method"),
                value_type: OtelAttributeType::String as u32,
                value: OtelAttributeValue {
                    string_value: sv("GET"),
                },
            },
            OtelKeyValue {
                key: sv("http.status_code"),
                value_type: OtelAttributeType::Int64 as u32,
                value: OtelAttributeValue { int64_value: 200 },
            },
            OtelKeyValue {
                key: sv("cache.hit"),
                value_type: OtelAttributeType::Bool as u32,
                value: OtelAttributeValue { bool_value: 1 },
            },
        ];
        b.iter_batched(
            || SpanGuard(start(tracer)),
            |guard| {
                let st = unsafe {
                    otel_span_add_event(guard.0, sv("request"), attrs.as_ptr(), attrs.len())
                };
                black_box(st);
                guard
            },
            BatchSize::SmallInput,
        );
    });

    g.finish();

    // Teardown (not measured): drop the cached tracer, then shut down and destroy the SDK.
    unsafe {
        otel_tracer_destroy(tracer);
        otel_sdk_shutdown(sdk, 2_000);
        otel_sdk_destroy(sdk);
    }
}

criterion_group!(benches, bench_sdk_backed);
criterion_main!(benches);
