/*
    The benchmark results:
    criterion = "0.5.1"

    Hardware: Apple M4 Pro
    Total Number of Cores:	10
    (Inside multipass vm running Ubuntu 22.04)
     
    // When no listener (automatic disable via RAII guard)
    | Test                        | Average time|
    |-----------------------------|-------------|
    | User_Event_4_Attributes     | 19.2 ns     |
    | User_Event_6_Attributes     | 19.6 ns     |
    | User_Event_4_Attributes_EventName_Custom | 20.2 ns |
    | User_Event_4_Attributes_EventName_FromLogRecord | 20.6 ns |

    // When listener is enabled (automatic enable via RAII guard)
    // The benchmark now automatically enables/disables the listener
    // using the commands below internally:
    //  echo 1 | sudo tee /sys/kernel/debug/tracing/events/user_events/myprovider_L2K1/enable
    //  echo 0 | sudo tee /sys/kernel/debug/tracing/events/user_events/myprovider_L2K1/enable
    | Test                        | Average time|
    |-----------------------------|-------------|
    | User_Event_4_Attributes     | 530 ns      |
    | User_Event_6_Attributes     | 586 ns      |
    | User_Event_4_Attributes_EventName_Custom | 590 ns |
    | User_Event_4_Attributes_EventName_FromLogRecord | 595 ns |
*/

// running the following from the current directory
// sudo -E ~/.cargo/bin/cargo bench --bench logs --all-features

use criterion::{criterion_group, criterion_main, Criterion};
use opentelemetry_appender_tracing::layer as tracing_layer;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::Resource;
#[cfg(feature = "experimental_eventname_callback")]
use opentelemetry_user_events_logs::EventNameCallback;
use opentelemetry_user_events_logs::Processor;
use tracing::error;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Registry;
use std::process::Command;

/// RAII guard for enabling/disabling user events listener
/// 
/// This guard automatically enables the user events listener when created and
/// disables it when dropped, ensuring proper cleanup even if the benchmark
/// panics or exits early.
/// 
/// The guard tracks whether the listener was already enabled before it was
/// created, and only disables it on drop if it wasn't already enabled.
/// This prevents interfering with other tests or processes that might have
/// enabled the listener.
struct UserEventsListenerGuard {
    provider_name: String,
    was_enabled: bool,
}

impl UserEventsListenerGuard {
    /// Enable the user events listener for the given provider
    /// 
    /// This method:
    /// 1. Checks if the listener is already enabled
    /// 2. Enables it if it's not already enabled
    /// 3. Returns a guard that will disable the listener on drop (if it wasn't already enabled)
    /// 
    /// # Arguments
    /// * `provider_name` - The name of the provider to enable/disable
    /// 
    /// # Returns
    /// * `Ok(Self)` - A guard that will disable the listener on drop
    /// * `Err(String)` - Error message if enabling failed
    fn enable(provider_name: &str) -> Result<Self, String> {
        let enable_path = format!("/sys/kernel/debug/tracing/events/user_events/{}_L2K1/enable", provider_name);
        
        // Check if already enabled
        let check_output = Command::new("sudo")
            .arg("cat")
            .arg(&enable_path)
            .output()
            .map_err(|e| format!("Failed to check listener status: {e}"))?;
        
        let was_enabled = check_output.status.success() && 
            String::from_utf8_lossy(&check_output.stdout).trim() == "1";
        
        // Enable the listener
        let enable_output = Command::new("sudo")
            .arg("sh")
            .arg("-c")
            .arg(format!("echo 1 | sudo tee {}", enable_path))
            .output()
            .map_err(|e| format!("Failed to enable listener: {e}"))?;
        
        if !enable_output.status.success() {
            return Err(format!(
                "Failed to enable listener. Error: {}",
                String::from_utf8_lossy(&enable_output.stderr)
            ));
        }
        
        println!("User events listener enabled for provider: {}", provider_name);
        
        Ok(UserEventsListenerGuard {
            provider_name: provider_name.to_string(),
            was_enabled,
        })
    }
    
    /// Check if user events are available on the system
    /// 
    /// This method checks if the user_events subsystem is available by
    /// reading from `/sys/kernel/tracing/user_events_status`.
    /// 
    /// # Returns
    /// * `Ok(String)` - The status content if user events are available
    /// * `Err(String)` - Error message if user events are not available
    fn check_user_events_available() -> Result<String, String> {
        let output = Command::new("sudo")
            .arg("cat")
            .arg("/sys/kernel/tracing/user_events_status")
            .output()
            .map_err(|e| format!("Failed to execute command: {e}"))?;

        if output.status.success() {
            let status = String::from_utf8_lossy(&output.stdout);
            Ok(status.to_string())
        } else {
            Err(format!(
                "Command executed with failing error code: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }
}

impl Drop for UserEventsListenerGuard {
    fn drop(&mut self) {
        let disable_path = format!("/sys/kernel/debug/tracing/events/user_events/{}_L2K1/enable", self.provider_name);
        
        // Only disable if it wasn't already enabled
        if !self.was_enabled {
            let disable_output = Command::new("sudo")
                .arg("sh")
                .arg("-c")
                .arg(format!("echo 0 | sudo tee {}", disable_path))
                .output();
            
            match disable_output {
                Ok(output) if output.status.success() => {
                    println!("User events listener disabled for provider: {}", self.provider_name);
                }
                Ok(output) => {
                    eprintln!("Failed to disable listener. Error: {}", 
                        String::from_utf8_lossy(&output.stderr));
                }
                Err(e) => {
                    eprintln!("Failed to disable listener: {}", e);
                }
            }
        } else {
            println!("User events listener was already enabled, leaving enabled for provider: {}", self.provider_name);
        }
    }
}

#[cfg(feature = "experimental_eventname_callback")]
struct EventNameFromLogRecordEventName;

#[cfg(feature = "experimental_eventname_callback")]
impl EventNameCallback for EventNameFromLogRecordEventName {
    #[inline(always)]
    fn get_name(&self, record: &opentelemetry_sdk::logs::SdkLogRecord) -> &'static str {
        record.event_name().unwrap_or("Log")
    }
}

#[cfg(feature = "experimental_eventname_callback")]
struct EventNameFromLogRecordCustom;

#[cfg(feature = "experimental_eventname_callback")]
impl EventNameCallback for EventNameFromLogRecordCustom {
    #[inline(always)]
    fn get_name(&self, record: &opentelemetry_sdk::logs::SdkLogRecord) -> &'static str {
        match record.event_name() {
            Some(name) if name.starts_with("Checkout") => "CheckoutEvent",
            Some(name) if name.starts_with("Payment") => "PaymentEvent",
            Some(_) => "OtherEvent",
            None => "DefaultEvent",
        }
    }
}

fn setup_provider_default() -> SdkLoggerProvider {
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

#[cfg(feature = "experimental_eventname_callback")]
fn setup_provider_with_callback<C>(event_name_callback: C) -> SdkLoggerProvider
where
    C: EventNameCallback + 'static,
{
    let user_event_processor = Processor::builder("myprovider")
        .with_event_name_callback(event_name_callback)
        .build()
        .unwrap();

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
    // Check if user events are available
    if let Err(e) = UserEventsListenerGuard::check_user_events_available() {
        eprintln!("Warning: User events not available: {}", e);
        eprintln!("Benchmarks will run without listener enabled");
    }
    
    // Enable listener with RAII guard
    let _guard = UserEventsListenerGuard::enable("myprovider")
        .unwrap_or_else(|e| {
            eprintln!("Warning: Failed to enable listener: {}", e);
            eprintln!("Benchmarks will run without listener enabled");
            // Return a dummy guard that does nothing on drop
            UserEventsListenerGuard {
                provider_name: "dummy".to_string(),
                was_enabled: true,
            }
        });
    
    let provider = setup_provider_default();
    let ot_layer = tracing_layer::OpenTelemetryTracingBridge::new(&provider);
    let subscriber = Registry::default().with(ot_layer);

    tracing::subscriber::with_default(subscriber, || {
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
    });
}

#[cfg(feature = "experimental_eventname_callback")]
fn benchmark_4_attributes_event_name_custom(c: &mut Criterion) {
    // Check if user events are available
    if let Err(e) = UserEventsListenerGuard::check_user_events_available() {
        eprintln!("Warning: User events not available: {}", e);
        eprintln!("Benchmarks will run without listener enabled");
    }
    
    // Enable listener with RAII guard
    let _guard = UserEventsListenerGuard::enable("myprovider")
        .unwrap_or_else(|e| {
            eprintln!("Warning: Failed to enable listener: {}", e);
            eprintln!("Benchmarks will run without listener enabled");
            // Return a dummy guard that does nothing on drop
            UserEventsListenerGuard {
                provider_name: "dummy".to_string(),
                was_enabled: true,
            }
        });
    
    let provider = setup_provider_with_callback(EventNameFromLogRecordCustom);
    let ot_layer = tracing_layer::OpenTelemetryTracingBridge::new(&provider);
    let subscriber = Registry::default().with(ot_layer);

    tracing::subscriber::with_default(subscriber, || {
        c.bench_function("User_Event_4_Attributes_EventName_Custom", |b| {
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
    });
}

#[cfg(feature = "experimental_eventname_callback")]
fn benchmark_4_attributes_event_name_from_log_record(c: &mut Criterion) {
    // Check if user events are available
    if let Err(e) = UserEventsListenerGuard::check_user_events_available() {
        eprintln!("Warning: User events not available: {}", e);
        eprintln!("Benchmarks will run without listener enabled");
    }
    
    // Enable listener with RAII guard
    let _guard = UserEventsListenerGuard::enable("myprovider")
        .unwrap_or_else(|e| {
            eprintln!("Warning: Failed to enable listener: {}", e);
            eprintln!("Benchmarks will run without listener enabled");
            // Return a dummy guard that does nothing on drop
            UserEventsListenerGuard {
                provider_name: "dummy".to_string(),
                was_enabled: true,
            }
        });
    
    let provider = setup_provider_with_callback(EventNameFromLogRecordEventName);
    let ot_layer = tracing_layer::OpenTelemetryTracingBridge::new(&provider);
    let subscriber = Registry::default().with(ot_layer);

    tracing::subscriber::with_default(subscriber, || {
        c.bench_function("User_Event_4_Attributes_EventName_FromLogRecord", |b| {
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
    });
}

fn benchmark_6_attributes(c: &mut Criterion) {
    // Check if user events are available
    if let Err(e) = UserEventsListenerGuard::check_user_events_available() {
        eprintln!("Warning: User events not available: {}", e);
        eprintln!("Benchmarks will run without listener enabled");
    }
    
    // Enable listener with RAII guard
    let _guard = UserEventsListenerGuard::enable("myprovider")
        .unwrap_or_else(|e| {
            eprintln!("Warning: Failed to enable listener: {}", e);
            eprintln!("Benchmarks will run without listener enabled");
            // Return a dummy guard that does nothing on drop
            UserEventsListenerGuard {
                provider_name: "dummy".to_string(),
                was_enabled: true,
            }
        });
    
    let provider = setup_provider_default();
    let ot_layer = tracing_layer::OpenTelemetryTracingBridge::new(&provider);
    let subscriber = Registry::default().with(ot_layer);

    tracing::subscriber::with_default(subscriber, || {
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
    });
}

fn criterion_benchmark(c: &mut Criterion) {
    benchmark_4_attributes(c);
    benchmark_6_attributes(c);
    #[cfg(feature = "experimental_eventname_callback")]
    benchmark_4_attributes_event_name_custom(c);
    #[cfg(feature = "experimental_eventname_callback")]
    benchmark_4_attributes_event_name_from_log_record(c);
}

criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = criterion_benchmark
}
criterion_main!(benches);
