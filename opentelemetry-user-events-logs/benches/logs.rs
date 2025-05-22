/*
    The benchmark results:
    criterion = "0.5.1"

    Hardware: Apple M4 Pro
    Total Number of Cores:	10
    (Inside multipass vm running Ubuntu 22.04)
    // When no listener
    | Test                        | Average time|
    |-----------------------------|-------------|
    | User_Event_4_Attributes     | 8 ns        |
    | User_Event_6_Attributes     | 8 ns        |

    // When listener is enabled
    // Run below to enable
    //  echo 1 | sudo tee /sys/kernel/debug/tracing/events/user_events/myprovider_L2K1/enable
    // Run below to disable
    //  echo 0 | sudo tee /sys/kernel/debug/tracing/events/user_events/myprovider_L2K1/enable
    | Test                        | Average time|
    |-----------------------------|-------------|
    | User_Event_4_Attributes     | 530 ns      |
    | User_Event_6_Attributes     | 586 ns      |
*/

// running the following from the current directory
// sudo -E ~/.cargo/bin/cargo bench --bench logs --all-features

use criterion::{criterion_group, criterion_main, Criterion};
use opentelemetry_appender_tracing::layer as tracing_layer;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::Resource;
use opentelemetry_user_events_logs::Processor;
use tracing::error;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Registry;

fn benchmark_with_ot_layer(c: &mut Criterion, name: &str, num_attributes: usize) {
    let user_event_processor = Processor::builder("myprovider").build().unwrap();
    let provider = SdkLoggerProvider::builder()
        .with_resource(
            Resource::builder_empty()
                .with_service_name("benchmark")
                .build(),
        )
        .with_log_processor(user_event_processor)
        .build();
    let ot_layer = tracing_layer::OpenTelemetryTracingBridge::new(&provider);
    let subscriber = Registry::default().with(ot_layer);

    tracing::subscriber::with_default(subscriber, || {
        c.bench_function(name, |b| {
            b.iter(|| {
                if num_attributes == 4 {
                    error!(
                        name : "CheckoutFailed",
                        field1 = "field1",
                        field2 = "field2",
                        field3 = "field3",
                        field4 = "field4",
                        message = "Unable to process checkout."
                    );
                } else if num_attributes == 6 {
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
                }
            });
        });
    });
}

fn criterion_benchmark(c: &mut Criterion) {
    benchmark_with_ot_layer(c, "User_Event_4_Attributes", 4);
    benchmark_with_ot_layer(c, "User_Event_6_Attributes", 6)
}

criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = criterion_benchmark
}
criterion_main!(benches);
