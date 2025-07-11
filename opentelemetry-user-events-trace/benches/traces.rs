/*
    The benchmark results:
    criterion = "0.5.1"

    Hardware: Apple M4 Pro
    Total Number of Cores:	10
    (Inside multipass vm running Ubuntu 22.04)
    // When no listener
    | Test                        | Average time|
    |-----------------------------|-------------|
    | SimpleSpan                  | 160 ns      |


    // When listener is enabled
    // Run below to enable
    //  echo 1 | sudo tee /sys/kernel/debug/tracing/events/user_events/opentelemetry_traces_L4K1/enable
    // Run below to disable
    //  echo 0 | sudo tee /sys/kernel/debug/tracing/events/user_events/opentelemetry_traces_L4K1/enable
    | Test                        | Average time|
    |-----------------------------|-------------|
    | SimpleSpan                  | 160 ns      |
*/

// running the following from the current directory
// sudo -E ~/.cargo/bin/cargo bench --bench traces

use criterion::{criterion_group, criterion_main, Criterion};
use opentelemetry::trace::{Span, Tracer, TracerProvider};
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_user_events_trace::UserEventsTracerProviderBuilderExt;

fn setup_provider() -> SdkTracerProvider {
    SdkTracerProvider::builder()
        .with_resource(
            opentelemetry_sdk::Resource::builder()
                .with_service_name("user-events-trace-example")
                .build(),
        )
        .with_user_events_exporter("opentelemetry_traces")
        .build()
}

fn benchmark_simple_span(c: &mut Criterion) {
    let provider = setup_provider();
    let tracer = provider.tracer("user-events-tracer");

    c.bench_function("SimpleSpan", |b| {
        b.iter(|| {
            let mut span = tracer
                .span_builder("my-span-name")
                .with_attributes([
                    opentelemetry::KeyValue::new("key1", "value1"),
                    opentelemetry::KeyValue::new("key2", "value2"),
                    opentelemetry::KeyValue::new("key3", "value3"),
                    opentelemetry::KeyValue::new("key4", "value4"),
                    opentelemetry::KeyValue::new("key5", "value5"),
                    opentelemetry::KeyValue::new("key6", "value6"),
                ])
                .start(&tracer);
            span.end();
        });
    });
}

fn criterion_benchmark(c: &mut Criterion) {
    benchmark_simple_span(c);
}

criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = criterion_benchmark
}
criterion_main!(benches);
