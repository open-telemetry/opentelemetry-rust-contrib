//! Run this stress test using `$ sudo -E ~/.cargo/bin/cargo run --bin user_events --release -- <num-of-threads>`.
//!
//! IMPORTANT:
//!     To test with `user_events` enabled, perform the following step before running the test:
//!     - Add `1` to `/sys/kernel/debug/tracing/events/user_events/testprovider_L4K1Gtestprovider/enable`:
//!         `echo 1 > /sys/kernel/debug/tracing/events/user_events/testprovider_L4K1Gtestprovider/enable`
//!     To test with `user_events` disabled, perform the following step:
//!     - Add `0` to `/sys/kernel/debug/tracing/events/user_events/testprovider_L4K1Gtestprovider/enable`:
//!         `echo 0 > /sys/kernel/debug/tracing/events/user_events/testprovider_L4K1Gtestprovider/enable`
//!
//!
// Conf - AMD EPYC 7763 64-Core Processor 2.44 GHz, 64GB RAM, Cores:8 , Logical processors: 16
// Stress Test Results (user_events disabled)
// Threads: 1 - Average Throughput: 42,086,520 iterations/sec
// Threads: 5 - Average Throughput: 35,767,375 iterations/sec
// Threads: 10 - Average Throughput: 29,189,340 iterations/sec
// Threads: 16 - Average Throughput: 19,579,138 iterations/sec

// Stress Test Results (user_events enabled)
// Threads: 1 - Average Throughput: 285,692 iterations/sec
// Threads: 5 - Average Throughput: 392,906 iterations/sec
// Threads: 10 - Average Throughput: 349,334 iterations/sec
// Threads: 16 - Average Throughput: 297,232 iterations/sec

use opentelemetry_appender_tracing::layer;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_user_events_logs::UserEventsLoggerProviderBuilderExt;
use tracing::info;
use tracing_subscriber::{prelude::*, EnvFilter};
mod throughput;

// Function to initialize the logger
fn init_logger() -> SdkLoggerProvider {
    SdkLoggerProvider::builder()
        .with_user_event_exporter("provider_name")
        .build()
}

// Function that performs the logging task
fn log_event_task() {
    info!(
        name = "my-event-name",
        event_id = 20,
        user_name = "otel user",
        user_email = "otel@opentelemetry.io"
    );
}

fn main() {
    // Initialize the logger
    let logger_provider = init_logger();
    let filter_otel = EnvFilter::new("info").add_directive("opentelemetry=off".parse().unwrap());
    let layer = layer::OpenTelemetryTracingBridge::new(&logger_provider);
    let layer = layer.with_filter(filter_otel);
    tracing_subscriber::registry().with(layer).init();

    // Use the provided stress test framework
    println!("Starting stress test for UserEventsExporter...");
    throughput::test_throughput(|| {
        log_event_task();
    });
    println!("Stress test completed.");
}
