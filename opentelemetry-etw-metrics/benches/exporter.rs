//! run with `$ cargo bench --bench exporter -- --exact <test_name>` to run specific test for logs
//! So to run test named "fibonacci" you would run `$ cargo bench --bench exporter -- --exact fibonacci`
//! To run all tests for logs you would run `$ cargo bench --bench exporter`
//!
/*
The benchmark results:
criterion = "0.5.1"
OS:
Hardware:
RAM:
| Test                           | Average time|
|--------------------------------|-------------|
|                                |             |
*/

use opentelemetry_etw_metrics::MetricsExporter;
use opentelemetry_sdk::{metrics::{data::{ResourceMetrics, ScopeMetrics}, exporter::PushMetricExporter}, Resource};

use criterion::{criterion_group, criterion_main, Criterion};

fn export() {
    let exporter = MetricsExporter::new();
    let mut resource_metrics = ResourceMetrics {
        resource: Resource::default(),
        scope_metrics: vec![ScopeMetrics::default(), ScopeMetrics::default()],
    };

    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        exporter.export(&mut resource_metrics).await.unwrap();
    });
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("export", |b| b.iter(|| { export()}));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
