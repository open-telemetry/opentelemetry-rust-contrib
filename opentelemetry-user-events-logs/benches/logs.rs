/*
    The benchmark results:
    criterion = "0.5.1"

    Hardware: <Hardware specifications>
    Total Number of Cores: <number>
    (Environment details)
    
    // When no listener (automatically set by benchmark)
    | Test                              | Average time |
    |-----------------------------------|--------------|
    | User_Event_4_Attributes_Disabled  | X ns         |
    | User_Event_6_Attributes_Disabled  | X ns         |

    // When listener is enabled (automatically set by benchmark)
    | Test                              | Average time |
    |-----------------------------------|--------------|
    | User_Event_4_Attributes_Enabled   | X ns         |
    | User_Event_6_Attributes_Enabled   | X ns         |
    
    // Fallback tests (when user_events is not available)
    | Test                        | Average time |
    |-----------------------------|--------------|
    | User_Event_4_Attributes     | X ns         |
    | User_Event_6_Attributes     | X ns         |

    Note: The benchmark now automatically enables and disables the user-events listener.
    If user_events is not available on the system, it falls back to the original test names.
*/

// running the following from the current directory
// sudo -E ~/.cargo/bin/cargo bench --bench logs --all-features

use criterion::{criterion_group, criterion_main, Criterion};
use opentelemetry_appender_tracing::layer as tracing_layer;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::Resource;
use opentelemetry_user_events_logs::Processor;
use std::fs;
use std::io::Write;
use std::process::Command;
use tracing::error;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Registry;

/// Attempts to enable or disable the user_events listener
/// Returns true if the operation was successful, false otherwise
fn set_user_events_listener(enabled: bool) -> bool {
    let enable_path = "/sys/kernel/debug/tracing/events/user_events/myprovider_L2K1/enable";
    let value = if enabled { "1" } else { "0" };
    
    // First try direct write (if we have permissions)
    if let Ok(mut file) = fs::OpenOptions::new().write(true).open(enable_path) {
        if file.write_all(value.as_bytes()).is_ok() {
            return true;
        }
    }
    
    // Fallback to using sudo with echo command
    let output = Command::new("sudo")
        .args(["tee", enable_path])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .and_then(|mut child| {
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(value.as_bytes()).ok();
            }
            child.wait()
        });
    
    match output {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

/// Checks if user_events is available on the system
fn is_user_events_available() -> bool {
    fs::metadata("/sys/kernel/debug/tracing/events/user_events/myprovider_L2K1/enable").is_ok()
}

fn setup_provider() -> SdkLoggerProvider {
    let user_event_processor = Processor::builder("myprovider").build().unwrap();
    SdkLoggerProvider::builder()
        .with_resource(
            Resource::builder_empty()
                .with_service_name("benchmark")
                .build(),
        )
        .with_log_processor(user_event_processor)
        .build()
}

fn benchmark_4_attributes(c: &mut Criterion) {
    let provider = setup_provider();
    let ot_layer = tracing_layer::OpenTelemetryTracingBridge::new(&provider);
    let subscriber = Registry::default().with(ot_layer);

    tracing::subscriber::with_default(subscriber, || {
        // Test with listener disabled
        if is_user_events_available() {
            set_user_events_listener(false);
            c.bench_function("User_Event_4_Attributes_Disabled", |b| {
                b.iter(|| {
                    error!(
                        name : "CheckoutFailed",
                        field1 = "field1",
                        field2 = "field2",
                        field3 = "field3",
                        field4 = "field4",
                        message = "Unable to process checkout."
                    );
                });
            });

            // Test with listener enabled
            set_user_events_listener(true);
            c.bench_function("User_Event_4_Attributes_Enabled", |b| {
                b.iter(|| {
                    error!(
                        name : "CheckoutFailed",
                        field1 = "field1",
                        field2 = "field2",
                        field3 = "field3",
                        field4 = "field4",
                        message = "Unable to process checkout."
                    );
                });
            });

            // Cleanup: disable listener
            set_user_events_listener(false);
        } else {
            // Fallback for systems without user_events support
            c.bench_function("User_Event_4_Attributes", |b| {
                b.iter(|| {
                    error!(
                        name : "CheckoutFailed",
                        field1 = "field1",
                        field2 = "field2",
                        field3 = "field3",
                        field4 = "field4",
                        message = "Unable to process checkout."
                    );
                });
            });
        }
    });
}

fn benchmark_6_attributes(c: &mut Criterion) {
    let provider = setup_provider();
    let ot_layer = tracing_layer::OpenTelemetryTracingBridge::new(&provider);
    let subscriber = Registry::default().with(ot_layer);

    tracing::subscriber::with_default(subscriber, || {
        // Test with listener disabled
        if is_user_events_available() {
            set_user_events_listener(false);
            c.bench_function("User_Event_6_Attributes_Disabled", |b| {
                b.iter(|| {
                    error!(
                        name : "CheckoutFailed",
                        field1 = "field1",
                        field2 = "field2",
                        field3 = "field3",
                        field4 = "field4",
                        field5 = "field5",
                        field6 = "field6",
                        message = "Unable to process checkout."
                    );
                });
            });

            // Test with listener enabled
            set_user_events_listener(true);
            c.bench_function("User_Event_6_Attributes_Enabled", |b| {
                b.iter(|| {
                    error!(
                        name : "CheckoutFailed",
                        field1 = "field1",
                        field2 = "field2",
                        field3 = "field3",
                        field4 = "field4",
                        field5 = "field5",
                        field6 = "field6",
                        message = "Unable to process checkout."
                    );
                });
            });

            // Cleanup: disable listener
            set_user_events_listener(false);
        } else {
            // Fallback for systems without user_events support
            c.bench_function("User_Event_6_Attributes", |b| {
                b.iter(|| {
                    error!(
                        name : "CheckoutFailed",
                        field1 = "field1",
                        field2 = "field2",
                        field3 = "field3",
                        field4 = "field4",
                        field5 = "field5",
                        field6 = "field6",
                        message = "Unable to process checkout."
                    );
                });
            });
        }
    });
}

fn criterion_benchmark(c: &mut Criterion) {
    benchmark_4_attributes(c);
    benchmark_6_attributes(c);
}

criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = criterion_benchmark
}
criterion_main!(benches);
