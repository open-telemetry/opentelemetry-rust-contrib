/*
    The benchmark results:
    criterion = "0.5.1"

    Hardware: <Hardware specifications>
    Total Number of Cores: <number>
    (Environment details)
    
    // When no listener (automatically set by benchmark)
    | Test                              | Average time |
    |-----------------------------------|--------------|
    | User_Events/4_Attributes_Disabled | X ns         |
    | User_Events/6_Attributes_Disabled | X ns         |

    // When listener is enabled (automatically set by benchmark)
    | Test                              | Average time |
    |-----------------------------------|--------------|
    | User_Events/4_Attributes_Enabled  | X ns         |
    | User_Events/6_Attributes_Enabled  | X ns         |

    Note: The benchmark automatically enables and disables the user-events listener
    to compare performance between the two states.
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

/// Helper function to emit a log event with the specified number of attributes
fn emit_log_event(attribute_count: u8) {
    match attribute_count {
        4 => {
            error!(
                name : "CheckoutFailed",
                field1 = "field1",
                field2 = "field2",
                field3 = "field3",
                field4 = "field4",
                message = "Unable to process checkout."
            );
        }
        6 => {
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
        }
        _ => panic!("Unsupported attribute count: {}", attribute_count),
    }
}

fn benchmark_user_events(c: &mut Criterion) {
    let mut group = c.benchmark_group("User_Events");
    
    let provider = setup_provider();
    let ot_layer = tracing_layer::OpenTelemetryTracingBridge::new(&provider);
    let subscriber = Registry::default().with(ot_layer);

    tracing::subscriber::with_default(subscriber, || {
        // Test configurations: (attribute_count, listener_enabled)
        let test_configs = [
            (4, false),
            (4, true),
            (6, false),
            (6, true),
        ];

        for (attribute_count, listener_enabled) in test_configs {
            let test_name = format!(
                "{}_Attributes_{}",
                attribute_count,
                if listener_enabled { "Enabled" } else { "Disabled" }
            );

            set_user_events_listener(listener_enabled);
            group.bench_function(&test_name, |b| {
                b.iter(|| emit_log_event(attribute_count));
            });
        }

        // Cleanup: disable listener
        set_user_events_listener(false);
    });
    
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = benchmark_user_events
}
criterion_main!(benches);
