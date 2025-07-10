/*
    The benchmark results:
    criterion = "0.5.1"

    Hardware: AMD EPYC 7763 64-Core Processor 2.44 GHz, 64GB RAM, Cores:8 , Logical processors: 16
    Total Number of Cores:	16
    // When no listener
    | Test                        | Average time|
    |-----------------------------|-------------|
    | Etw_4_Attributes            | 8 ns        |
    | Etw_6_Attributes            | 8 ns        |

    // When listener is enabled
    | Test                        | Average time|
    |-----------------------------|-------------|
    | Etw_4_Attributes            | 1.3659 µs   |
    | Etw_6_Attributes            | 1.6487 µs   |
*/

// running the following from the current directory
// cargo bench --bench logs --all-features

use criterion::{criterion_group, criterion_main, Criterion};
use opentelemetry_appender_tracing::layer as tracing_layer;
use opentelemetry_etw_logs::Processor;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::Resource;
use tracing::error;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Registry;

fn setup_otel_provider() -> SdkLoggerProvider {
    let etw_processor = Processor::builder("provider_name").build().unwrap();
    SdkLoggerProvider::builder()
        .with_resource(
            Resource::builder_empty()
                .with_service_name("benchmark")
                .build(),
        )
        .with_log_processor(etw_processor)
        .build()
}

fn benchmark_with_ot_layer_4_attributes(c: &mut Criterion) {
    let provider = setup_otel_provider();
    let ot_layer = tracing_layer::OpenTelemetryTracingBridge::new(&provider);
    let subscriber = Registry::default().with(ot_layer);

    tracing::subscriber::with_default(subscriber, || {
        c.bench_function("Etw_4_Attributes", |b| {
            b.iter(|| {
                error!(
                    name : "CheckoutFailed",
                    field1 = "field1",
                    field2 = "field2",
                    field3 = "field3",
                    field4 = "field4",
                    message = "Unable to process checkout."
                );
            });
        });
    });
}

fn benchmark_with_ot_layer_6_attributes(c: &mut Criterion) {
    let provider = setup_otel_provider();
    let ot_layer = tracing_layer::OpenTelemetryTracingBridge::new(&provider);
    let subscriber = Registry::default().with(ot_layer);

    tracing::subscriber::with_default(subscriber, || {
        c.bench_function("Etw_6_Attributes", |b| {
            b.iter(|| {
                error!(
                    name : "CheckoutFailed",
                    field1 = "field1",
                    field2 = "field2",
                    field3 = "field3",
                    field4 = "field4",
                    field5 = "field5",
                    field6 = "field6",
                    message = "Unable to process checkout."
                );
            });
        });
    });
}

fn criterion_benchmark(c: &mut Criterion) {
    benchmark_with_ot_layer_4_attributes(c);
    benchmark_with_ot_layer_6_attributes(c);
}

criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = criterion_benchmark
}
criterion_main!(benches);
