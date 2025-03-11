//! Run this stress test using `cargo run --bin etw --release -- <num-of-threads>`.
//!
// Conf - AMD EPYC 7763 64-Core Processor 2.44 GHz, 64GB RAM, Cores:8 , Logical processors: 16
// Stress Test Results (no listener)
// Threads: 1 - Average Throughput: 52M iterations/sec
// Threads: 5 - Average Throughput: 250M iterations/sec
// Threads: 10 - Average Throughput: 320M iterations/sec
// Threads: 16 - Average Throughput: 400M iterations/sec

// Stress Test Results (logman is listening)
// Threads: 1 - Average Throughput: 600K iterations/sec
// Threads: 5 - Average Throughput: 2.7M iterations/sec
// Threads: 10 - Average Throughput: 4.1M iterations/sec
// Threads: 16 - Average Throughput: 5M iterations/sec
// $EtwSessionGuid = (new-object System.Diagnostics.Tracing.EventSource("my-provider-name")).Guid.ToString()`
// logman create trace OtelETWExampleBasic -o OtelETWExampleBasic.log -p "{$EtwSessionGuid}" -f bincirc -max 1000
// logman start OtelETWExampleBasic
// RUN test here...
// logman stop OtelETWExampleBasic

use opentelemetry_appender_tracing::layer;
use opentelemetry_etw_logs::{ExporterConfig, ReentrantLogProcessor};
use opentelemetry_sdk::logs::SdkLoggerProvider;
use std::collections::HashMap;
use tracing::info;
use tracing_subscriber::prelude::*;
mod throughput;

// Function to initialize the logger
fn init_logger() -> SdkLoggerProvider {
    let exporter_config = ExporterConfig {
        default_keyword: 1,
        keywords_map: HashMap::new(),
    };
    let reenterant_processor = ReentrantLogProcessor::new(
        "my-provider-name",
        "my-event-name".into(),
        None,
        exporter_config,
    );
    SdkLoggerProvider::builder()
        .with_log_processor(reenterant_processor)
        .build()
}

fn main() {
    let logger_provider = init_logger();
    let layer = layer::OpenTelemetryTracingBridge::new(&logger_provider);
    tracing_subscriber::registry().with(layer).init();

    throughput::test_throughput(|| {
        info!(
            name : "my-event-name",
            event_id = 20,
            user_name = "otel user",
            user_email = "otel@opentelemetry.io"
        );
    });

    println!("Stress test completed.");
}
