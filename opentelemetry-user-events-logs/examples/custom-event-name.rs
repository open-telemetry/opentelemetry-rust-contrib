//! Example demonstrating how to use a custom event name callback
//! Run with `$ cargo run --example custom-event-name --all-features

use opentelemetry_appender_tracing::layer;
use opentelemetry_sdk::logs::LoggerProviderBuilder;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_user_events_logs::Processor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{thread, time::Duration};
use tracing::{debug, error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

fn init_logger() -> SdkLoggerProvider {
    let filter_fmt = EnvFilter::new("info").add_directive("opentelemetry=debug".parse().unwrap());
    let fmt_layer = tracing_subscriber::fmt::layer().with_filter(filter_fmt);
    let _guard = tracing_subscriber::registry().with(fmt_layer).set_default(); // Temporary subscriber active for this function

    // Create a user_events processor with a custom event name callback
    // This callback will use the event name from the log record, or
    // derive a name based on severity if none is provided
    let user_event_processor = Processor::builder("myprovider")
        .with_event_name_callback(|record| {
            // If an event name is provided, use it
            if let Some(name) = record.event_name() {
                return name;
            }

            // Otherwise, create a name based on severity
            if let Some(severity) = record.severity_number() {
                match severity {
                    opentelemetry::logs::Severity::Error => "ErrorEvent",
                    opentelemetry::logs::Severity::Warn => "WarningEvent",
                    opentelemetry::logs::Severity::Info => "InfoEvent",
                    opentelemetry::logs::Severity::Debug => "DebugEvent",
                    opentelemetry::logs::Severity::Fatal => "FatalEvent",
                    _ => "UnknownEvent",
                }
            } else {
                "DefaultEvent"
            }
        })
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
    let filter_fmt = EnvFilter::new("debug").add_directive("opentelemetry=debug".parse().unwrap());
    let fmt_layer = tracing_subscriber::fmt::layer().with_filter(filter_fmt);

    tracing_subscriber::registry()
        .with(otel_layer)
        .with(fmt_layer)
        .init();

    println!("Emitting logs with different event names...");
    println!("To capture these events with perf, run in a separate terminal:");
    println!("sudo perf record -e user_events:myprovider_L1K1,user_events:myprovider_L2K1,user_events:myprovider_L3K1,user_events:myprovider_L4K1,user_events:myprovider_L5K1");
    println!("Press Ctrl+C to exit");

    // run in a loop to ensure that tracepoints are not removed from kernel fs
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    let mut counter = 0;
    while running.load(Ordering::SeqCst) {
        counter += 1;

        // Log with explicit event name
        error!(
            name: "ExplicitErrorName",
            event_id = 20,
            user_name = "otel user",
            user_email = "otel@opentelemetry.io",
            message = "This is an error message with explicit name",
            counter = counter,
        );

        // Log without event name, will use callback to derive name based on severity
        error!(
            message = "This is an error message with derived name",
            counter = counter,
        );

        warn!(
            message = "This is a warning message with derived name",
            counter = counter,
        );

        info!(
            message = "This is an info message with derived name",
            counter = counter,
        );

        debug!(
            message = "This is a debug message with derived name",
            counter = counter,
        );

        thread::sleep(Duration::from_secs(1));
    }

    let _ = logger_provider.shutdown();
}
