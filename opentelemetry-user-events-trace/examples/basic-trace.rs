//! run with `$ cargo run --example basic-trace --all-features

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
    let filter_fmt = EnvFilter::new("info").add_directive("opentelemetry=debug".parse().unwrap());
    let fmt_layer = tracing_subscriber::fmt::layer().with_filter(filter_fmt);
    let _guard = tracing_subscriber::registry().with(fmt_layer).set_default(); // Temporary subscriber active for this function

    let provider = SdkTracerProvider::builder()
        .with_user_event_exporter("myprovider")
        .build();
    global::set_tracer_provider(provider.clone());
    provider
}

fn main() {
    // OpenTelemetry layer with a filter to ensure OTel's own logs are not fed back into
    // the OpenTelemetry pipeline.
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
        // event_id is passed as an attribute now, there is nothing in metadata where a
        // numeric id can be stored.
        let mut span = tracer
            .span_builder("my-event-name")
            .with_attributes(vec![opentelemetry::KeyValue::new("my-key", "my-value")])
            .start(&tracer);
        span.end();
        thread::sleep(Duration::from_secs(1));
    }
    let _ = tracer_provider.shutdown();
}
