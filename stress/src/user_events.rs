//! Run this stress test using `$ sudo -E ~/.cargo/bin/cargo run --bin user_events --release -- <num-of-threads>`.
//!
//! IMPORTANT:
//! To test with `user_events` enabled, perform the following step before running the test:
//!   - Add `1` to `/sys/kernel/debug/tracing/events/user_events/testprovider_L4K1Gtestprovider/enable`:
//!     `echo 1 > /sys/kernel/debug/tracing/events/user_events/testprovider_L4K1Gtestprovider/enable`
//! To test with `user_events` disabled, perform the following step:
//!   - Add `0` to `/sys/kernel/debug/tracing/events/user_events/testprovider_L4K1Gtestprovider/enable`:
//!     `echo 0 > /sys/kernel/debug/tracing/events/user_events/testprovider_L4K1Gtestprovider/enable`
//!
//! NOTE: Running as `sudo -E` ensures the environment variables for Rust and Cargo are retained,
//! and you have sufficient permissions to access `/sys/kernel/debug/tracing` and utilize `user_events`.

use opentelemetry_appender_tracing::layer;
use opentelemetry_sdk::logs::LoggerProvider;
use opentelemetry_user_events_logs::{ExporterConfig, ReentrantLogProcessor, UserEventsExporter};
use std::collections::HashMap;
use tracing::error;
use tracing_subscriber::prelude::*;
mod throughput;

// Function to initialize the logger
fn init_logger() -> LoggerProvider {
    let exporter_config = ExporterConfig {
        default_keyword: 1,
        keywords_map: HashMap::new(),
    };
    let exporter = UserEventsExporter::new("test", None, exporter_config);
    let reentrant_processor = ReentrantLogProcessor::new(exporter);
    LoggerProvider::builder()
        .with_log_processor(reentrant_processor)
        .build()
}

// Function that performs the logging task
fn log_event_task() {
    error!(
        name = "my-event-name",
        event_id = 20,
        user_name = "otel user",
        user_email = "otel@opentelemetry.io"
    );
}

fn main() {
    // Initialize the logger
    let logger_provider = init_logger();
    let layer = layer::OpenTelemetryTracingBridge::new(&logger_provider);
    tracing_subscriber::registry().with(layer).init();

    // Use the provided stress test framework
    println!("Starting stress test for UserEventsExporter...");
    throughput::test_throughput(|| {
        log_event_task(); // Log the error event in each iteration
    });
    println!("Stress test completed.");
}
