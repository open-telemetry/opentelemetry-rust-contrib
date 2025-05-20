//! run with `$ cargo run --example basic-logs --all-features

use opentelemetry_appender_tracing::layer;
use opentelemetry_sdk::logs::LoggerProviderBuilder;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_user_events_logs::Processor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{thread, time::Duration};
use tracing::error;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

fn init_logger() -> SdkLoggerProvider {
    let filter_fmt = EnvFilter::new("info").add_directive("opentelemetry=debug".parse().unwrap());
    let fmt_layer = tracing_subscriber::fmt::layer().with_filter(filter_fmt);
    let _guard = tracing_subscriber::registry().with(fmt_layer).set_default(); // Temporary subscriber active for this function

    let user_event_processor = Processor::builder("myprovider")
        .build()
        .unwrap_or_else(|err| {
            eprintln!("Failed to create processor: {}", err);
            panic!("exiting due to error during initialization");
        });
    LoggerProviderBuilder::default()
        .with_log_processor(user_event_processor)
        .build()
}

fn main() {
    // OpenTelemetry layer with a filter to ensure OTel's own logs are not fed back into
    // the OpenTelemetry pipeline.
    let filter_otel = EnvFilter::new("info").add_directive("opentelemetry=off".parse().unwrap());
    let logger_provider = init_logger();
    let otel_layer = layer::OpenTelemetryTracingBridge::new(&logger_provider);
    let otel_layer = otel_layer.with_filter(filter_otel);

    // Create a new tracing::Fmt layer to print the logs to stdout. It has a
    // default filter of `info` level and above, and `debug` and above for logs
    // from OpenTelemetry crates. The filter levels can be customized as needed.
    let filter_fmt = EnvFilter::new("info").add_directive("opentelemetry=debug".parse().unwrap());
    let fmt_layer = tracing_subscriber::fmt::layer().with_filter(filter_fmt);

    tracing_subscriber::registry()
        .with(otel_layer)
        .with(fmt_layer)
        .init();

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
        error!(
            name: "my-event-name",
            event_id = 20,
            user_name = "otel user",
            user_email = "otel@opentelemetry.io",
            message = "This is a test message",
        );
        thread::sleep(Duration::from_secs(1));
    }
    let _ = logger_provider.shutdown();
}
