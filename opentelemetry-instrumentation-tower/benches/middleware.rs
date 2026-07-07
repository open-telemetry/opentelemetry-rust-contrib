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
//! Each scenario configures the layer via `with_tracing`/`with_metrics`, and
//! where a real backend is needed it is installed as the OpenTelemetry global
//! provider before the layer is built.
//!
//! ## Known Performance Characteristics
//!
//! ### Middleware-internal (optimizable by this crate)
//!
//! - **Heap allocations per request**: `path`, `URI` and (for non-common
//!   methods or schemes) the `method`/`scheme` strings still `to_string()`
//!   / `to_owned()` into a new `String`. The `method` and `scheme` labels
//!   are promoted to `&'static str` for the common HTTP methods
//!   (GET/POST/PUT/DELETE/HEAD/OPTIONS/PATCH/CONNECT/TRACE) and the `http`
//!   and `https` schemes, so the corresponding `KeyValue::clone()` is
//!   allocation-free in the hot path.
//! - **`Vec<KeyValue>` allocations**: `span_attributes` and `label_superset`
//!   are built and populated on every request.
//! - **Unconditional attribute extraction**: all `KeyValue` attributes are built
//!   on every request without first checking whether the tracer or meter are
//!   no-ops.
//!
//! ### Tower / OTel SDK (outside this middleware's control)
//!
//! - **`Box::pin`** wrapping the response future — heap-allocates on every
//!   request. This is a consequence of the Tower `Service` trait design.
//!   (See [PR #561](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/561)
//!   for a `ResponseFuture`-based alternative.)
//! - **`global::get_text_map_propagator`** — acquires a `RwLock` on every
//!   request. Caching would require a new `with_propagator` builder method
//!   since there is no public API to obtain an owned/`Arc`'d propagator from
//!   the global.
//! - **`OtelContext::current()`** — thread-local access + clone in
//!   `finalize_request`.
//!
//! ## Run
//!
//! ```sh
//! cargo bench --bench middleware -p opentelemetry-instrumentation-tower
//! ```
//!
//! ## Reference Numbers
//!
//! Latest measurements (criterion median):
//!
//! | Scenario             | Median   | vs baseline |
//! | -------------------- | -------- | ----------- |
//! | baseline             |   44 ns  | —           |
//! | noop                 |  568 ns  | +524 ns     |
//! | tracing              |  708 ns  | +664 ns     |
//! | tracing-sampled-out  |  592 ns  | +548 ns     |
//! | metrics              |  895 ns  | +851 ns     |
//! | tracing + metrics    | 1054 ns  | +1010 ns    |
//!
//! Captured on: MacBook Pro, Apple M4 Pro (10P + 4E cores), 24 GB RAM,
//! macOS 26.4.1, rustc 1.95.0, OpenTelemetry 0.32.
//!

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use opentelemetry::global;
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

/// Build requests outside the timed loop so request construction cost does not
/// inflate the middleware overhead measurement.
fn build_requests(n: u64) -> Vec<http::Request<String>> {
    (0..n)
        .map(|_| {
            http::Request::builder()
                .method("GET")
                .uri("http://example.com/users/123")
                .body(String::new())
                .unwrap()
        })
        .collect()
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
            let requests = build_requests(iters);

            let start = std::time::Instant::now();
            for req in requests {
                let resp = service.ready().await.unwrap().call(req).await.unwrap();
                black_box(resp);
            }
            start.elapsed()
        });
    });

    // Scenario 2: Middleware present, but both tracer and meter are no-ops.
    // Measures the pure overhead of the HTTPLayer machinery (attribute extraction,
    // context propagation hooks, etc.) when no real telemetry is produced.
    group.bench_function(BenchmarkId::new("request", "noop"), |b| {
        let layer = HTTPLayerBuilder::builder()
            .with_tracing(false)
            .with_metrics(false)
            .build()
            .unwrap();
        b.to_async(&rt).iter_custom(|iters| {
            let layer = layer.clone();
            async move {
                let mut service = ServiceBuilder::new()
                    .layer(layer)
                    .service(tower::service_fn(handler));
                let requests = build_requests(iters);

                let start = std::time::Instant::now();
                for req in requests {
                    let resp = service.ready().await.unwrap().call(req).await.unwrap();
                    black_box(resp);
                }
                start.elapsed()
            }
        });
    });

    // Scenario 3: Tracing only (no-op meter)
    group.bench_function(BenchmarkId::new("request", "tracing"), |b| {
        // No exporter = no-op processing, just measures instrumentation overhead.
        global::set_tracer_provider(SdkTracerProvider::builder().build());
        let layer = HTTPLayerBuilder::builder()
            .with_metrics(false)
            .build()
            .unwrap();
        b.to_async(&rt).iter_custom(|iters| {
            let layer = layer.clone();
            async move {
                let mut service = ServiceBuilder::new()
                    .layer(layer)
                    .service(tower::service_fn(handler));
                let requests = build_requests(iters);

                let start = std::time::Instant::now();
                for req in requests {
                    let resp = service.ready().await.unwrap().call(req).await.unwrap();
                    black_box(resp);
                }
                start.elapsed()
            }
        });
    });

    // Scenario 4: Tracing with AlwaysOff sampler (no-op meter)
    group.bench_function(BenchmarkId::new("request", "tracing-sampled-out"), |b| {
        global::set_tracer_provider(
            SdkTracerProvider::builder()
                .with_sampler(Sampler::AlwaysOff)
                .build(),
        );
        let layer = HTTPLayerBuilder::builder()
            .with_metrics(false)
            .build()
            .unwrap();
        b.to_async(&rt).iter_custom(|iters| {
            let layer = layer.clone();
            async move {
                let mut service = ServiceBuilder::new()
                    .layer(layer)
                    .service(tower::service_fn(handler));
                let requests = build_requests(iters);

                let start = std::time::Instant::now();
                for req in requests {
                    let resp = service.ready().await.unwrap().call(req).await.unwrap();
                    black_box(resp);
                }
                start.elapsed()
            }
        });
    });

    // Scenario 5: Metrics only (no-op tracer)
    group.bench_function(BenchmarkId::new("request", "metrics"), |b| {
        let _metric_exporter = InMemoryMetricExporter::default();
        // Use very long interval to ensure no timer-based exports during benchmark.
        let reader = PeriodicReader::builder(_metric_exporter.clone())
            .with_interval(Duration::from_secs(3600))
            .build();
        global::set_meter_provider(SdkMeterProvider::builder().with_reader(reader).build());
        let layer = HTTPLayerBuilder::builder()
            .with_tracing(false)
            .build()
            .unwrap();
        b.to_async(&rt).iter_custom(|iters| {
            let layer = layer.clone();
            async move {
                let mut service = ServiceBuilder::new()
                    .layer(layer)
                    .service(tower::service_fn(handler));
                let requests = build_requests(iters);

                let start = std::time::Instant::now();
                for req in requests {
                    let resp = service.ready().await.unwrap().call(req).await.unwrap();
                    black_box(resp);
                }
                start.elapsed()
            }
        });
    });

    // Scenario 6: Both tracing + metrics
    group.bench_function(BenchmarkId::new("request", "tracing+metrics"), |b| {
        // No exporter = no-op processing, just measures instrumentation overhead.
        global::set_tracer_provider(SdkTracerProvider::builder().build());
        let _metric_exporter = InMemoryMetricExporter::default();
        // Use very long interval to ensure no timer-based exports during benchmark.
        let reader = PeriodicReader::builder(_metric_exporter.clone())
            .with_interval(Duration::from_secs(3600))
            .build();
        global::set_meter_provider(SdkMeterProvider::builder().with_reader(reader).build());
        let layer = HTTPLayerBuilder::builder().build().unwrap();
        b.to_async(&rt).iter_custom(|iters| {
            let layer = layer.clone();
            async move {
                let mut service = ServiceBuilder::new()
                    .layer(layer)
                    .service(tower::service_fn(handler));
                let requests = build_requests(iters);

                let start = std::time::Instant::now();
                for req in requests {
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
