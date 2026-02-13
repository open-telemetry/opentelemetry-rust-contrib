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
//! - **Tracing**: `HTTPLayer` with active tracer, no-op meter (all spans sampled)
//! - **Tracing (sampled-out)**: Same, but with `AlwaysOff` sampler (all spans dropped)
//! - **Metrics**: `HTTPLayer` with active meter, no-op tracer (no spans created)
//! - **Tracing + Metrics**: `HTTPLayer` with both active tracer and active meter
//!
//! Since tower's `HTTPLayer` always creates both tracer and meter instruments from
//! global providers, the "tracing only" scenario uses a no-op global meter (the default
//! when no meter provider is set), making metric instruments effectively free.
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
//! | Baseline                    | ~189 ns   | -                    |
//! | Tracing                     | ~1120 ns  | +~931 ns             |
//! | Tracing (sampled-out)       | ~1001 ns  | +~812 ns             |
//! | Metrics                     | ~1313 ns  | +~1124 ns            |
//! | Tracing + Metrics           | ~1414 ns  | +~1225 ns            |

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

/// Simple handler to simulate a real HTTP endpoint
async fn handler(req: http::Request<String>) -> Result<http::Response<String>, Infallible> {
    Ok(http::Response::new(format!("Path: {}", req.uri().path())))
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

    // Scenario 2: Tracing only (no-op meter via default global)
    group.bench_function(BenchmarkId::new("request", "tracing"), |b| {
        b.to_async(&rt).iter_custom(|iters| async move {
            let _provider = setup_tracer();
            // Global meter is not set, so HTTPLayer's meter instruments are no-op

            let layer = HTTPLayerBuilder::builder().build().unwrap();
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
        });
    });

    // Scenario 3: Tracing with AlwaysOff sampler (measures non-sampled path overhead)
    group.bench_function(BenchmarkId::new("request", "tracing-sampled-out"), |b| {
        b.to_async(&rt).iter_custom(|iters| async move {
            let _provider = setup_sampled_out_tracer();
            // Global meter is not set, so HTTPLayer's meter instruments are no-op

            let layer = HTTPLayerBuilder::builder().build().unwrap();
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
        });
    });

    // Scenario 4: Metrics only (no-op tracer via default global)
    group.bench_function(BenchmarkId::new("request", "metrics"), |b| {
        b.to_async(&rt).iter_custom(|iters| async move {
            let (_meter_provider, _metric_exporter) = setup_meter();
            // Global tracer is not set, so HTTPLayer's tracer produces no-op spans

            let layer = HTTPLayerBuilder::builder().build().unwrap();
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
        });
    });

    // Scenario 5: Both tracing + metrics
    group.bench_function(BenchmarkId::new("request", "tracing+metrics"), |b| {
        b.to_async(&rt).iter_custom(|iters| async move {
            let _tracer_provider = setup_tracer();
            let (_meter_provider, _metric_exporter) = setup_meter();

            let layer = HTTPLayerBuilder::builder().build().unwrap();
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
        });
    });

    group.finish();
}

criterion_group!(benches, benchmark_middleware);
criterion_main!(benches);
