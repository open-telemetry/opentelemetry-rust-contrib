//! Run this stress test using `$ sudo -E ~/.cargo/bin/cargo run --bin user_events --release -- <num-of-threads>`.
//!
//! IMPORTANT:
//!     To test with `user_events` enabled, perform the following step before running the test:
//!     - Add `1` to `/sys/kernel/debug/tracing/events/user_events/myprovider_L4K1/enable`:
//!         echo 1 | sudo tee /sys/kernel/debug/tracing/events/user_events/myprovider_L2K1/enable
//!     To test with `user_events` disabled, perform the following step:
//!     - Add `0` to `/sys/kernel/debug/tracing/events/user_events/myprovider_L4K1/enable`:
//!         echo 0 | sudo tee /sys/kernel/debug/tracing/events/user_events/myprovider_L2K1/enable
//!
//!
// Conf - AMD EPYC 7763 64-Core Processor 2.44 GHz, 64GB RAM, Cores:8 , Logical processors: 16
// Stress Test Results (user_events disabled)
// Threads: 1 - Average Throughput: 43M iterations/sec
// Threads: 5 - Average Throughput: 185M iterations/sec
// Threads: 10 - Average Throughput: 250M iterations/sec
// Threads: 16 - Average Throughput: 320M iterations/sec

// Hardware: Apple M4 Pro
// Total Number of Cores:	10
// (Inside multipass vm running Ubuntu 22.04)
// Stress Test Results (user_events enabled)
// Threads: 1 - Average Throughput: TODO
// Threads: 5 - Average Throughput: TODO
// Threads: 10 - Average Throughput: 1.7 M iterations/sec
// Threads: 16 - Average Throughput: TODO
//
// Stress Test Results (user_events disabled)
// Threads: 1 - Average Throughput: TODO
// Threads: 5 - Average Throughput: TODO
// Threads: 10 - Average Throughput: 1.1 B iterations/sec
// Threads: 16 - Average Throughput: TODO

use opentelemetry_appender_tracing::layer;
use opentelemetry_sdk::logs::LoggerProviderBuilder;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_user_events_logs::Processor;
use tracing::error;
use tracing_subscriber::{prelude::*, EnvFilter};
mod throughput;

// Function to initialize the logger
fn init_logger() -> SdkLoggerProvider {
    let user_event_processor = Processor::builder("myprovider").build().unwrap();
    LoggerProviderBuilder::default()
        .with_log_processor(user_event_processor)
        .build()
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
        error!(
            name : "my-event-name",
            event_id = 20,
            user_name = "otel user",
            user_email = "otel@opentelemetry.io"
        );
    });
    println!("Stress test completed.");
}
