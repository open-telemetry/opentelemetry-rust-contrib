//! Run with `cargo run --bin etw_traces --release -- <num-threads>`.
//!
//! Use an ETW session for provider `stress_traces` to measure with a listener
//! enabled. Run with several thread counts, such as 1, 2, 4, 8, and the number
//! of logical processors, to verify throughput scales with available CPUs.

use opentelemetry::trace::{Span, Tracer, TracerProvider};
use opentelemetry_etw_traces::Processor;
use opentelemetry_sdk::trace::SdkTracerProvider;

mod throughput;

fn main() {
    let processor = Processor::builder("stress_traces").build().unwrap();
    let provider = SdkTracerProvider::builder()
        .with_span_processor(processor)
        .build();
    let tracer = provider.tracer("stress");

    println!("Starting stress test for ETW traces...");
    throughput::test_throughput(move || {
        let mut span = tracer.start("stress-span");
        span.end();
    });
    println!("Stress test completed.");
}
