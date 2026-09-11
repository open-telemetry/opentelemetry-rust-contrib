//! Run with `sudo -E cargo run --bin user_events_traces --release -- <num-threads>`.
//!
//! To measure with a listener enabled:
//! `echo 1 | sudo tee /sys/kernel/debug/tracing/events/user_events/stress_traces_L4K1/enable`
//!
//! Run with several thread counts, such as 1, 2, 4, 8, and the number of
//! logical processors, to verify throughput scales with available CPUs.

use opentelemetry::trace::{Span, Tracer, TracerProvider};
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_user_events_trace::Processor;

mod throughput;

fn main() {
    let processor = Processor::builder("stress_traces").build().unwrap();
    let provider = SdkTracerProvider::builder()
        .with_span_processor(processor)
        .build();
    let tracer = provider.tracer("stress");

    println!("Starting stress test for user_events traces...");
    throughput::test_throughput(move || {
        let mut span = tracer.start("stress-span");
        span.end();
    });
    println!("Stress test completed.");
}
