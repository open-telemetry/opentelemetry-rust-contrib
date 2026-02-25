//! Benchmarks for tower OpenTelemetry middleware overhead.
//!
//! This benchmark measures the **pure instrumentation overhead** of the `HTTPLayer`
//! middleware. It uses tower's service utilities to call handlers directly, bypassing
//! the network stack (TCP/HTTP parsing). This isolates the middleware cost and provides
//! consistent, reproducible measurements.
//!
//! **Purpose**: Track instrumentation performance over time and identify regressions.
//! The absolute numbers are less important than relative changes between versions.
//!
//! ## Scenarios
//!
//! - **Baseline**: No middleware (control measurement)
//! - **No-op**: `HTTPLayer` present, but both tracer and meter are no-ops
//! - **Tracing**: `HTTPLayer` with active tracer, no-op meter (all spans sampled)
//! - **Tracing (sampled-out)**: Same, but with `AlwaysOff` sampler (all spans dropped)
//! - **Metrics**: `HTTPLayer` with active meter, no-op tracer (no spans created)
//! - **Tracing + Metrics**: `HTTPLayer` with both active tracer and active meter
//!
//! Each tracing scenario sets the global `TracerProvider` before building the layer so
//! that no tracer state leaks between scenarios.  Scenarios that do not need traces use
//! `NoopTracerProvider` (a true no-op) to reset the global.  No meter reset is needed
//! because the global meter is never set in tracing-only scenarios, leaving meter
//! instruments as no-ops by default.
//!
//! ## Known Performance Issues (tracked for future improvement)
//!
//! The `noop` scenario (~960 ns) reveals ~830 ns of fixed middleware overhead that runs
//! unconditionally regardless of whether real telemetry is produced. Root causes:
//!
//! - **Heap allocations**: `scheme`, `method`, `path`, `URI`, `protocol`, `version` each
//!   `to_string()` / `to_owned()` into a new `String` per request.
//! - **`Vec<KeyValue>` allocations**: `span_attributes` and `label_superset` built and
//!   populated on every request.
//! - **`Box::pin`** wrapping the response future — heap-allocates on every request.
//! - **`global::get_text_map_propagator`** — acquires a `RwLock` on every request.
//!   Caching would require a new `with_propagator` builder method since there is no
//!   public API to obtain an owned/`Arc`'d propagator from the global.
//! - **`OtelContext::current()`** — thread-local access + clone in `finalize_request`.
//! - **Unconditional attribute extraction**: all `KeyValue` attributes (span attrs,
//!   metric label superset, custom extractors) are built on every request without
//!   first checking whether the tracer or meter are no-ops and would discard them
//!   immediately anyway.
//!
//! ## Run
//!
//! ```sh
//! cargo bench --bench middleware -p opentelemetry-instrumentation-tower
//! ```
//!
//! ## Results (Apple M4 Pro)
//!
//! | Scenario                    | Latency   | Overhead vs Baseline |
//! |-----------------------------|-----------|----------------------|
//! | Baseline                    | ~131 ns   | -                    |
//! | No-op                       | ~936 ns   | +~805 ns             |
//! | Tracing                     | ~1100 ns  | +~969 ns             |
//! | Tracing (sampled-out)       | ~999 ns   | +~868 ns             |
//! | Metrics                     | ~1295 ns  | +~1164 ns            |
//! | Tracing + Metrics           | ~1436 ns  | +~1305 ns            |

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use opentelemetry::global;
use opentelemetry::trace::noop::NoopTracerProvider;
use opentelemetry_instrumentation_tower::HTTPLayerBuilder;
use opentelemetry_sdk::{
    metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider},
    trace::{Sampler, SdkTracerProvider},
};
use std::convert::Infallible;
use std::hint::black_box;
use std::time::Duration;
use tower::{Service, ServiceBuilder, ServiceExt};

/// Minimal handler — returns an empty body to keep baseline noise as low as possible.
async fn handler(_req: http::Request<String>) -> Result<http::Response<String>, Infallible> {
    Ok(http::Response::new(String::new()))
}

/// Setup tracer provider with no-op processor (measures instrumentation overhead only)
fn setup_tracer() -> SdkTracerProvider {
    // No exporter = no-op processing, just measures instrumentation overhead
    let provider = SdkTracerProvider::builder().build();
    global::set_tracer_provider(provider.clone());
    provider
}

/// Setup tracer provider with AlwaysOff sampler (all spans are dropped).
/// This measures the overhead of the non-sampled code path.
fn setup_sampled_out_tracer() -> SdkTracerProvider {
    let provider = SdkTracerProvider::builder()
        .with_sampler(Sampler::AlwaysOff)
        .build();
    global::set_tracer_provider(provider.clone());
    provider
}

/// Reset global tracer provider to a true no-op instance.
/// Call this in scenarios that should not generate traces.
fn noop_tracer() {
    global::set_tracer_provider(NoopTracerProvider::new());
}

/// Setup meter provider with in-memory exporter (minimal I/O overhead)
fn setup_meter() -> (SdkMeterProvider, InMemoryMetricExporter) {
    let exporter = InMemoryMetricExporter::default();
    // Use very long interval to ensure no timer-based exports during benchmark
    let reader = PeriodicReader::builder(exporter.clone())
        .with_interval(Duration::from_secs(3600))
        .build();
    let provider = SdkMeterProvider::builder().with_reader(reader).build();
    global::set_meter_provider(provider.clone());
    (provider, exporter)
}

fn benchmark_middleware(c: &mut Criterion) {
    // Use tokio runtime since Criterion's AsyncExecutor is implemented for tokio
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("tower-instrumentation");
    group.throughput(Throughput::Elements(1));

    // Scenario 1: Baseline - no middleware
    group.bench_function(BenchmarkId::new("request", "baseline"), |b| {
        b.to_async(&rt).iter_custom(|iters| async move {
            let mut service = tower::service_fn(handler);

            let start = std::time::Instant::now();
            for _ in 0..iters {
                let req = http::Request::builder()
                    .method("GET")
                    .uri("http://example.com/users/123")
                    .body(String::new())
                    .unwrap();
                let resp = service.ready().await.unwrap().call(req).await.unwrap();
                black_box(resp);
            }
            start.elapsed()
        });
    });

    // Scenario 2: Middleware present, but both tracer and meter are no-ops.
    // Measures the pure overhead of the HTTPLayer machinery (attribute extraction,
    // context propagation hooks, etc.) when no real telemetry is produced.
    // Ideally this should be very close to the baseline.
    group.bench_function(BenchmarkId::new("request", "noop"), |b| {
        noop_tracer(); // reset any tracer left from a previous run
                       // meter is not set, so meter instruments are already no-op
        let layer = HTTPLayerBuilder::builder().build().unwrap();
        b.to_async(&rt).iter_custom(|iters| {
            let layer = layer.clone();
            async move {
                let mut service = ServiceBuilder::new()
                    .layer(layer)
                    .service(tower::service_fn(handler));

                let start = std::time::Instant::now();
                for _ in 0..iters {
                    let req = http::Request::builder()
                        .method("GET")
                        .uri("http://example.com/users/123")
                        .body(String::new())
                        .unwrap();
                    let resp = service.ready().await.unwrap().call(req).await.unwrap();
                    black_box(resp);
                }
                start.elapsed()
            }
        });
    });

    // Scenario 3: Tracing only (global meter not set, so meter instruments are no-op)
    group.bench_function(BenchmarkId::new("request", "tracing"), |b| {
        let _tracer_provider = setup_tracer();
        let layer = HTTPLayerBuilder::builder().build().unwrap();
        b.to_async(&rt).iter_custom(|iters| {
            let layer = layer.clone();
            async move {
                let mut service = ServiceBuilder::new()
                    .layer(layer)
                    .service(tower::service_fn(handler));

                let start = std::time::Instant::now();
                for _ in 0..iters {
                    let req = http::Request::builder()
                        .method("GET")
                        .uri("http://example.com/users/123")
                        .body(String::new())
                        .unwrap();
                    let resp = service.ready().await.unwrap().call(req).await.unwrap();
                    black_box(resp);
                }
                start.elapsed()
            }
        });
    });

    // Scenario 4: Tracing with AlwaysOff sampler (global meter not set, so meter instruments are no-op)
    group.bench_function(BenchmarkId::new("request", "tracing-sampled-out"), |b| {
        let _tracer_provider = setup_sampled_out_tracer();
        let layer = HTTPLayerBuilder::builder().build().unwrap();
        b.to_async(&rt).iter_custom(|iters| {
            let layer = layer.clone();
            async move {
                let mut service = ServiceBuilder::new()
                    .layer(layer)
                    .service(tower::service_fn(handler));

                let start = std::time::Instant::now();
                for _ in 0..iters {
                    let req = http::Request::builder()
                        .method("GET")
                        .uri("http://example.com/users/123")
                        .body(String::new())
                        .unwrap();
                    let resp = service.ready().await.unwrap().call(req).await.unwrap();
                    black_box(resp);
                }
                start.elapsed()
            }
        });
    });

    // Scenario 5: Metrics only (tracer reset to NoopTracerProvider)
    group.bench_function(BenchmarkId::new("request", "metrics"), |b| {
        noop_tracer();
        let (_meter_provider, _metric_exporter) = setup_meter();
        let layer = HTTPLayerBuilder::builder().build().unwrap();
        b.to_async(&rt).iter_custom(|iters| {
            let layer = layer.clone();
            async move {
                let mut service = ServiceBuilder::new()
                    .layer(layer)
                    .service(tower::service_fn(handler));

                let start = std::time::Instant::now();
                for _ in 0..iters {
                    let req = http::Request::builder()
                        .method("GET")
                        .uri("http://example.com/users/123")
                        .body(String::new())
                        .unwrap();
                    let resp = service.ready().await.unwrap().call(req).await.unwrap();
                    black_box(resp);
                }
                start.elapsed()
            }
        });
    });

    // Scenario 6: Both tracing + metrics
    group.bench_function(BenchmarkId::new("request", "tracing+metrics"), |b| {
        let _tracer_provider = setup_tracer();
        let (_meter_provider, _metric_exporter) = setup_meter();
        let layer = HTTPLayerBuilder::builder().build().unwrap();
        b.to_async(&rt).iter_custom(|iters| {
            let layer = layer.clone();
            async move {
                let mut service = ServiceBuilder::new()
                    .layer(layer)
                    .service(tower::service_fn(handler));

                let start = std::time::Instant::now();
                for _ in 0..iters {
                    let req = http::Request::builder()
                        .method("GET")
                        .uri("http://example.com/users/123")
                        .body(String::new())
                        .unwrap();
                    let resp = service.ready().await.unwrap().call(req).await.unwrap();
                    black_box(resp);
                }
                start.elapsed()
            }
        });
    });

    group.finish();
}

criterion_group!(benches, benchmark_middleware);
criterion_main!(benches);
