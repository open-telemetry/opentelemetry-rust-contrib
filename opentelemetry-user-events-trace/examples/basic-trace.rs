//! run with `$ cargo run --example basic-trace`
//! to listen for events, as root:
//! $perf record -e user_events:myprovider_L4K1

use opentelemetry::global;
use opentelemetry::trace::Span;
use opentelemetry::trace::Tracer;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_user_events_trace::UserEventsTracerProviderBuilderExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{thread, time::Duration};

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

fn init_tracer() -> SdkTracerProvider {
    let provider = SdkTracerProvider::builder()
        .with_user_events_exporter("myprovider")
        .build();
    global::set_tracer_provider(provider.clone());
    provider
}

fn main() {
    let filter_fmt = EnvFilter::new("info").add_directive("opentelemetry=debug".parse().unwrap());
    let fmt_layer = tracing_subscriber::fmt::layer().with_filter(filter_fmt);
    tracing_subscriber::registry().with(fmt_layer).init();
    let tracer_provider = init_tracer();
    let tracer = global::tracer("user-events-tracer");
    // run in a loop to ensure that tracepoints are not removed from kernel fs

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    while running.load(Ordering::SeqCst) {
        let mut span = tracer
            .span_builder("my-event-name")
            .with_attributes([opentelemetry::KeyValue::new("my-key", "my-value")])
            .start(&tracer);
        span.end();
        thread::sleep(Duration::from_secs(1));
    }

    let status = tracer_provider.shutdown();
    if let Err(e) = status {
        println!("Error shutting down: {:?}", e);
    }
}
