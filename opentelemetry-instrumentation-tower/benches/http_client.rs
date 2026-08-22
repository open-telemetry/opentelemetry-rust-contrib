//! Benchmarks for the tower OpenTelemetry HTTP **client** layer overhead.
//!
//! This benchmark measures the **pure instrumentation overhead** of the
//! `http::client::Layer` middleware. The inner "transport" is a `service_fn`
//! that immediately returns a response, so the network stack is bypassed and
//! only the layer's cost (span creation, attribute extraction, context
//! injection, metric recording) is measured.
//!
//! **Purpose**: Track instrumentation performance over time and identify
//! regressions. The absolute numbers matter less than relative changes.
//!
//! ## Scenarios
//!
//! - **Baseline**: No layer (control measurement)
//! - **No-op**: `http::client::Layer` present, tracer and meter are no-ops
//! - **Tracing**: active tracer, no-op meter (includes context injection cost)
//! - **Tracing (sampled-out)**: `AlwaysOff` sampler (non-sampled code path)
//! - **Metrics**: active meter, no-op tracer
//! - **Tracing + Metrics**: both active
//!
//! Like the server benchmark, this runs with the default no-op text-map
//! propagator so the numbers reflect the layer's own overhead rather than the
//! cost of a concrete propagator's header injection.
//!
//! ## Run
//!
//! ```sh
//! cargo bench --bench http_client -p opentelemetry-instrumentation-tower
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use opentelemetry::global;
use opentelemetry_instrumentation_tower::http::client::LayerBuilder;
use opentelemetry_sdk::{
    metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider},
    trace::{Sampler, SdkTracerProvider},
};
use std::convert::Infallible;
use std::hint::black_box;
use std::time::Duration;
use tower::{Service, ServiceBuilder, ServiceExt};

/// Minimal transport — returns an empty response, standing in for a real client.
async fn transport(_req: http::Request<String>) -> Result<http::Response<String>, Infallible> {
    Ok(http::Response::new(String::new()))
}

/// Build requests outside the timed loop so request construction cost does not
/// inflate the layer overhead measurement.
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

/// Setup tracer provider with no-op processor (measures instrumentation overhead only).
fn setup_tracer() -> SdkTracerProvider {
    let provider = SdkTracerProvider::builder().build();
    global::set_tracer_provider(provider.clone());
    provider
}

/// Setup tracer provider with AlwaysOff sampler (all spans are dropped).
fn setup_sampled_out_tracer() -> SdkTracerProvider {
    let provider = SdkTracerProvider::builder()
        .with_sampler(Sampler::AlwaysOff)
        .build();
    global::set_tracer_provider(provider.clone());
    provider
}

/// Setup meter provider with in-memory exporter (minimal I/O overhead).
fn setup_meter() -> (SdkMeterProvider, InMemoryMetricExporter) {
    let exporter = InMemoryMetricExporter::default();
    let reader = PeriodicReader::builder(exporter.clone())
        .with_interval(Duration::from_secs(3600))
        .build();
    let provider = SdkMeterProvider::builder().with_reader(reader).build();
    global::set_meter_provider(provider.clone());
    (provider, exporter)
}

fn benchmark_http_client(c: &mut Criterion) {
    // Use tokio runtime since Criterion's AsyncExecutor is implemented for tokio
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("tower-http-client");
    group.throughput(Throughput::Elements(1));

    // Scenario 1: Baseline - no layer
    group.bench_function(BenchmarkId::new("request", "baseline"), |b| {
        b.to_async(&rt).iter_custom(|iters| async move {
            let mut service = tower::service_fn(transport);
            let requests = build_requests(iters);

            let start = std::time::Instant::now();
            for req in requests {
                let resp = service.ready().await.unwrap().call(req).await.unwrap();
                black_box(resp);
            }
            start.elapsed()
        });
    });

    // Scenario 2: Layer present, but tracing and metrics are disabled via the builder.
    group.bench_function(BenchmarkId::new("request", "noop"), |b| {
        let layer = LayerBuilder::builder()
            .with_tracing(false)
            .with_metrics(false)
            .build()
            .unwrap();
        b.to_async(&rt).iter_custom(|iters| {
            let layer = layer.clone();
            async move {
                let mut service = ServiceBuilder::new()
                    .layer(layer)
                    .service(tower::service_fn(transport));
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

    // Scenario 3: Tracing only (metrics disabled via the builder)
    group.bench_function(BenchmarkId::new("request", "tracing"), |b| {
        let _tracer_provider = setup_tracer();
        let layer = LayerBuilder::builder().with_metrics(false).build().unwrap();
        b.to_async(&rt).iter_custom(|iters| {
            let layer = layer.clone();
            async move {
                let mut service = ServiceBuilder::new()
                    .layer(layer)
                    .service(tower::service_fn(transport));
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

    // Scenario 4: Tracing with AlwaysOff sampler (metrics disabled via the builder)
    group.bench_function(BenchmarkId::new("request", "tracing-sampled-out"), |b| {
        let _tracer_provider = setup_sampled_out_tracer();
        let layer = LayerBuilder::builder().with_metrics(false).build().unwrap();
        b.to_async(&rt).iter_custom(|iters| {
            let layer = layer.clone();
            async move {
                let mut service = ServiceBuilder::new()
                    .layer(layer)
                    .service(tower::service_fn(transport));
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

    // Scenario 5: Metrics only (tracing disabled via the builder)
    group.bench_function(BenchmarkId::new("request", "metrics"), |b| {
        let (_meter_provider, _metric_exporter) = setup_meter();
        let layer = LayerBuilder::builder().with_tracing(false).build().unwrap();
        b.to_async(&rt).iter_custom(|iters| {
            let layer = layer.clone();
            async move {
                let mut service = ServiceBuilder::new()
                    .layer(layer)
                    .service(tower::service_fn(transport));
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
        let _tracer_provider = setup_tracer();
        let (_meter_provider, _metric_exporter) = setup_meter();
        let layer = LayerBuilder::builder().build().unwrap();
        b.to_async(&rt).iter_custom(|iters| {
            let layer = layer.clone();
            async move {
                let mut service = ServiceBuilder::new()
                    .layer(layer)
                    .service(tower::service_fn(transport));
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

criterion_group!(benches, benchmark_http_client);
criterion_main!(benches);
