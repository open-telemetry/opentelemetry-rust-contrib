//! Benchmarks for actix-web OpenTelemetry middleware overhead.
//!
//! This benchmark measures the **pure instrumentation overhead** of the tracing
//! and metrics middleware. It uses actix-web's test utilities to call handlers
//! directly, bypassing the network stack (TCP/HTTP parsing). This isolates the
//! middleware cost and provides consistent, reproducible measurements.
//!
//! **Purpose**: Track instrumentation performance over time and identify regressions.
//! The absolute numbers are less important than relative changes between versions.
//!
//! ## Scenarios
//!
//! - **Baseline**: No middleware (control measurement)
//! - **Tracing (sync)**: `RequestTracing` with `sync-middleware` feature
//! - **Tracing (no-sync)**: `RequestTracing` without `sync-middleware` feature
//! - **Tracing + Metrics**: Both middlewares combined
//!
//! ## Feature: `sync-middleware`
//!
//! The `sync-middleware` feature adds `.attach()` and `drop()` calls around the
//! service invocation to support synchronous code accessing the current span context.
//! This adds overhead. Compare "tracing-sync" vs "tracing-no-sync" to measure impact.
//!
//! ## Run
//!
//! ```sh
//! # With sync-middleware:
//! cargo bench --bench middleware --features "metrics,sync-middleware"
//!
//! # Without sync-middleware:
//! cargo bench --bench middleware --features "metrics"
//! ```
//!
//! ## Results (Apple M1 Pro)
//!
//! | Scenario                    | Latency   | Overhead vs Baseline |
//! |-----------------------------|-----------|----------------------|
//! | Baseline                    | ~600 ns   | -                    |
//! | Tracing (sync)              | ~1.29 µs  | +~690 ns             |
//! | Tracing (no-sync)           | ~1.26 µs  | +~660 ns             |
//! | Tracing (sync) + Metrics    | ~2.24 µs  | +~1.64 µs            |
//! | Tracing (no-sync) + Metrics | ~2.24 µs  | +~1.64 µs            |

use actix_web::{test, web, App, HttpResponse};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use opentelemetry::global;
use opentelemetry_instrumentation_actix_web::{RequestMetrics, RequestTracing};
use opentelemetry_sdk::{
    metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider},
    trace::SdkTracerProvider,
};
use std::hint::black_box;
use std::time::Duration;

/// Handler with path parameter to simulate real-world route matching
async fn user_handler(path: web::Path<u32>) -> HttpResponse {
    HttpResponse::Ok().body(format!("User: {}", path.into_inner()))
}

/// Setup tracer provider with no-op processor (measures instrumentation overhead only)
fn setup_tracer() -> SdkTracerProvider {
    // No exporter = no-op processing, just measures instrumentation overhead
    let provider = SdkTracerProvider::builder().build();
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

    let mut group = c.benchmark_group("actix-web-instrumentation");
    group.throughput(Throughput::Elements(1));

    // Scenario 1: Baseline - no middleware
    group.bench_function(BenchmarkId::new("request", "baseline"), |b| {
        b.to_async(&rt).iter_custom(|iters| async move {
            let app =
                test::init_service(App::new().route("/users/{id}", web::get().to(user_handler)))
                    .await;

            let start = std::time::Instant::now();
            for _ in 0..iters {
                let req = test::TestRequest::get().uri("/users/123").to_request();
                let resp = test::call_service(&app, req).await;
                black_box(resp);
            }
            start.elapsed()
        });
    });

    // Scenario 2: Tracing only (with sync-middleware if feature enabled)
    #[cfg(feature = "sync-middleware")]
    let tracing_label = "tracing-sync";
    #[cfg(not(feature = "sync-middleware"))]
    let tracing_label = "tracing-no-sync";

    group.bench_function(BenchmarkId::new("request", tracing_label), |b| {
        b.to_async(&rt).iter_custom(|iters| async move {
            let _provider = setup_tracer();

            let app = test::init_service(
                App::new()
                    .wrap(RequestTracing::new())
                    .route("/users/{id}", web::get().to(user_handler)),
            )
            .await;

            let start = std::time::Instant::now();
            for _ in 0..iters {
                let req = test::TestRequest::get().uri("/users/123").to_request();
                let resp = test::call_service(&app, req).await;
                black_box(resp);
            }
            start.elapsed()
        });
    });

    // Scenario 3: Both tracing + metrics (sync-middleware status matches tracing scenario)
    #[cfg(feature = "sync-middleware")]
    let combined_label = "tracing-sync+metrics";
    #[cfg(not(feature = "sync-middleware"))]
    let combined_label = "tracing-no-sync+metrics";

    group.bench_function(BenchmarkId::new("request", combined_label), |b| {
        b.to_async(&rt).iter_custom(|iters| async move {
            let _tracer_provider = setup_tracer();
            let (_meter_provider, _metric_exporter) = setup_meter();

            let app = test::init_service(
                App::new()
                    .wrap(RequestTracing::new())
                    .wrap(RequestMetrics::default())
                    .route("/users/{id}", web::get().to(user_handler)),
            )
            .await;

            let start = std::time::Instant::now();
            for _ in 0..iters {
                let req = test::TestRequest::get().uri("/users/123").to_request();
                let resp = test::call_service(&app, req).await;
                black_box(resp);
            }
            start.elapsed()
        });
    });

    group.finish();
}

criterion_group!(benches, benchmark_middleware);
criterion_main!(benches);
