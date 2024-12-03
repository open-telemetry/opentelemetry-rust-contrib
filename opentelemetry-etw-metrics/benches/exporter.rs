//! run with `$ cargo bench --bench exporter -- --exact <test_name>` to run specific test for logs
//! So to run test named "fibonacci" you would run `$ cargo bench --bench exporter -- --exact fibonacci`
//! To run all tests for logs you would run `$ cargo bench --bench exporter`
//!
/*
The benchmark results:
criterion = "0.5.1"
OS: Windows 11 Enterprise N, 23H2, Build 22631.4460
Hardware: Intel(R) Xeon(R) Platinum 8370C CPU @ 2.80GHz   2.79 GHz, 16vCPUs
RAM: 64.0 GB
| Test                           | Average time|
|--------------------------------|-------------|
| exporter                       | 847.38µs    |
*/

use opentelemetry::{metrics::MeterProvider as _, KeyValue};
use opentelemetry_etw_metrics::MetricsExporter;
use opentelemetry_sdk::{
    metrics::{reader::MetricReader, PeriodicReader, SdkMeterProvider},
    runtime, Resource,
};

use criterion::{criterion_group, criterion_main, Criterion};

fn export() {
    // Create a new tokio runtime that blocks on the async execution
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let exporter = MetricsExporter::new();
            let reader = PeriodicReader::builder(exporter, runtime::Tokio).build();
            let meter_provider = SdkMeterProvider::builder()
                .with_resource(Resource::new(vec![KeyValue::new(
                    "service.name",
                    "service-name",
                )]))
                .with_reader(reader.clone())
                .build();
            let meter = meter_provider.meter("etw-bench");
            let gauge = meter.u64_gauge("gauge").build();

            for _ in 0..10_000 {
                gauge.record(1, &[KeyValue::new("key", "value")]);
            }

            reader.force_flush().unwrap();
        });
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("export", |b| b.iter(|| export()));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
