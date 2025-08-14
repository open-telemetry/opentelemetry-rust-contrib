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
use std::fs;
use std::io::{self, Read};
use tracing::error;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Registry;

const PROVIDER_NAME: &str = "myprovider";
/// Suffix used by the kernel user_events provider naming in these benchmarks.
/// Documented to avoid magic strings in path construction.
const USER_EVENTS_PROVIDER_SUFFIX: &str = "_L2K1";
/// Provider name used for dummy guards that perform no operations.
/// This value is not used for actual provider operations, only for internal tracking.
const DUMMY_PROVIDER_NAME: &str = "dummy";

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
///
/// # Fields
/// * `provider_name` - The name of the kernel user events provider being managed by this guard.
///   This is used to construct sysfs paths and identify which provider's listener should be enabled or disabled.
///   It is essential for ensuring that the guard operates on the correct provider, especially when multiple
///   providers may exist or when running benchmarks that interact with different user events sources.
/// * `was_enabled` - Tracks whether the listener was already enabled before this guard was created.
///   This field is crucial for preventing the guard from disabling listeners that were enabled
///   by external processes or other parts of the system. When the guard is dropped, it only
///   disables the listener if `was_enabled` is false, ensuring that external processes that
///   may have enabled the listener for their own purposes are not disrupted.
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
        let enable_path = format!(
            "/sys/kernel/debug/tracing/events/user_events/{}{}/enable",
            provider_name, USER_EVENTS_PROVIDER_SUFFIX
        );

        // Check if already enabled by reading only the first byte of the file
        let was_enabled = match fs::File::open(&enable_path) {
            Ok(mut file) => {
                let mut buf = [0u8; 1];
                match file.read(&mut buf) {
                    Ok(0) => false, // empty file, treat as disabled
                    Ok(_) => buf[0] == b'1',
                    Err(e) => {
                        return Err(format!("Failed to read listener status: {}", e));
                    }
                }
            }
            Err(e) => {
                if e.kind() == io::ErrorKind::PermissionDenied {
                    return Err(format!(
                        "Insufficient permissions to read '{}'. Please run the benchmark as root or with appropriate capabilities (CAP_SYS_ADMIN). Error: {}",
                        enable_path, e
                    ));
                } else {
                    return Err(format!("Failed to check listener status: {}", e));
                }
            }
        };

        // Enable the listener by writing "1" to the enable file
        // Note: No newline needed since we only read the first byte when checking status
        if let Err(e) = fs::write(&enable_path, b"1") {
            if e.kind() == io::ErrorKind::PermissionDenied {
                return Err(format!(
                    "Insufficient permissions to write to '{}'. Please run the benchmark as root or with appropriate capabilities (CAP_SYS_ADMIN). Error: {}",
                    enable_path, e
                ));
            } else {
                return Err(format!("Failed to enable listener: {}", e));
            }
        }

        println!(
            "User events listener enabled for provider: {}",
            provider_name
        );

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
        match fs::read_to_string("/sys/kernel/tracing/user_events_status") {
            Ok(status) => Ok(status),
            Err(e) => {
                if e.kind() == io::ErrorKind::PermissionDenied {
                    Err(format!(
                        "Insufficient permissions to read '/sys/kernel/tracing/user_events_status'. Please run the benchmark as root or with appropriate capabilities (CAP_SYS_ADMIN). Error: {}",
                        e
                    ))
                } else if e.kind() == io::ErrorKind::NotFound {
                    Err("User events subsystem not available on this system".to_string())
                } else {
                    Err(format!("Failed to check user events availability: {}", e))
                }
            }
        }
    }

    /// Create a dummy guard that performs no action on drop
    ///
    /// This is used when enabling the listener fails; it ensures the
    /// benchmark can proceed without attempting to disable anything later.
    fn dummy_guard() -> Self {
        UserEventsListenerGuard {
            provider_name: DUMMY_PROVIDER_NAME.to_string(),
            was_enabled: true,
        }
    }
}

impl Drop for UserEventsListenerGuard {
    fn drop(&mut self) {
        let disable_path = format!(
            "/sys/kernel/debug/tracing/events/user_events/{}{}/enable",
            self.provider_name, USER_EVENTS_PROVIDER_SUFFIX
        );

        // Only disable if it wasn't already enabled
        if !self.was_enabled {
            match fs::write(&disable_path, b"0") {
                Ok(_) => {
                    eprintln!(
                        "User events listener disabled for provider: {}",
                        self.provider_name
                    );
                }
                Err(e) => {
                    if e.kind() == io::ErrorKind::PermissionDenied {
                        eprintln!(
                            "Failed to disable listener due to insufficient permissions. Please run the benchmark as root or with appropriate capabilities (CAP_SYS_ADMIN). Error: {}",
                            e
                        );
                    } else {
                        eprintln!("Failed to disable listener: {}", e);
                    }
                }
            }
        } else {
            eprintln!(
                "User events listener was already enabled, leaving enabled for provider: {}",
                self.provider_name
            );
        }
    }
}

/// Helper function to enable user events listener with fallback to dummy guard
///
/// This function attempts to enable the user events listener for the given provider.
/// If enabling fails, it prints a warning message and returns a dummy guard that
/// performs no operations. This ensures benchmarks can proceed even when the
/// listener cannot be enabled.
///
/// # Arguments
/// * `provider_name` - The name of the provider to enable
///
/// # Returns
/// * `UserEventsListenerGuard` - Either a real guard or a dummy guard
fn enable_listener_with_fallback(provider_name: &str) -> UserEventsListenerGuard {
    UserEventsListenerGuard::enable(provider_name).unwrap_or_else(|e| {
        eprintln!("Warning: Failed to enable listener: {}", e);
        eprintln!("Benchmarks will run without listener enabled");
        // Return a dummy guard that does nothing on drop
        UserEventsListenerGuard::dummy_guard()
    })
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
    let user_event_processor = Processor::builder(PROVIDER_NAME).build().unwrap();

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
    let user_event_processor = Processor::builder(PROVIDER_NAME)
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

    let mut group = c.benchmark_group("User_Event_4_Attributes");

    // Benchmark with listener disabled
    {
        let provider = setup_provider_default();
        let ot_layer = tracing_layer::OpenTelemetryTracingBridge::new(&provider);
        let subscriber = Registry::default().with(ot_layer);

        tracing::subscriber::with_default(subscriber, || {
            group.bench_function("disabled", |b| {
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

    // Benchmark with listener enabled
    {
        let provider = setup_provider_default();
        // Enable listener with RAII guard (after provider is built so tracepoints exist)
        let _guard = enable_listener_with_fallback(PROVIDER_NAME);
        let ot_layer = tracing_layer::OpenTelemetryTracingBridge::new(&provider);
        let subscriber = Registry::default().with(ot_layer);

        tracing::subscriber::with_default(subscriber, || {
            group.bench_function("enabled", |b| {
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

    group.finish();
}

#[cfg(feature = "experimental_eventname_callback")]
fn benchmark_4_attributes_event_name_custom(c: &mut Criterion) {
    // Check if user events are available
    if let Err(e) = UserEventsListenerGuard::check_user_events_available() {
        eprintln!("Warning: User events not available: {}", e);
        eprintln!("Benchmarks will run without listener enabled");
    }

    let mut group = c.benchmark_group("User_Event_4_Attributes_EventName_Custom");

    // Benchmark with listener disabled
    {
        let provider = setup_provider_with_callback(EventNameFromLogRecordCustom);
        let ot_layer = tracing_layer::OpenTelemetryTracingBridge::new(&provider);
        let subscriber = Registry::default().with(ot_layer);

        tracing::subscriber::with_default(subscriber, || {
            group.bench_function("disabled", |b| {
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

    // Benchmark with listener enabled
    {
        let provider = setup_provider_with_callback(EventNameFromLogRecordCustom);
        // Enable listener with RAII guard (after provider is built so tracepoints exist)
        let _guard = enable_listener_with_fallback(PROVIDER_NAME);
        let ot_layer = tracing_layer::OpenTelemetryTracingBridge::new(&provider);
        let subscriber = Registry::default().with(ot_layer);

        tracing::subscriber::with_default(subscriber, || {
            group.bench_function("enabled", |b| {
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

    group.finish();
}

#[cfg(feature = "experimental_eventname_callback")]
fn benchmark_4_attributes_event_name_from_log_record(c: &mut Criterion) {
    // Check if user events are available
    if let Err(e) = UserEventsListenerGuard::check_user_events_available() {
        eprintln!("Warning: User events not available: {}", e);
        eprintln!("Benchmarks will run without listener enabled");
    }

    let mut group = c.benchmark_group("User_Event_4_Attributes_EventName_FromLogRecord");

    // Benchmark with listener disabled
    {
        let provider = setup_provider_with_callback(EventNameFromLogRecordEventName);
        let ot_layer = tracing_layer::OpenTelemetryTracingBridge::new(&provider);
        let subscriber = Registry::default().with(ot_layer);

        tracing::subscriber::with_default(subscriber, || {
            group.bench_function("disabled", |b| {
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

    // Benchmark with listener enabled
    {
        let provider = setup_provider_with_callback(EventNameFromLogRecordEventName);
        // Enable listener with RAII guard (after provider is built so tracepoints exist)
        let _guard = enable_listener_with_fallback(PROVIDER_NAME);
        let ot_layer = tracing_layer::OpenTelemetryTracingBridge::new(&provider);
        let subscriber = Registry::default().with(ot_layer);

        tracing::subscriber::with_default(subscriber, || {
            group.bench_function("enabled", |b| {
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

    group.finish();
}

fn benchmark_6_attributes(c: &mut Criterion) {
    // Check if user events are available
    if let Err(e) = UserEventsListenerGuard::check_user_events_available() {
        eprintln!("Warning: User events not available: {}", e);
        eprintln!("Benchmarks will run without listener enabled");
    }

    let mut group = c.benchmark_group("User_Event_6_Attributes");

    // Benchmark with listener disabled
    {
        let provider = setup_provider_default();
        let ot_layer = tracing_layer::OpenTelemetryTracingBridge::new(&provider);
        let subscriber = Registry::default().with(ot_layer);

        tracing::subscriber::with_default(subscriber, || {
            group.bench_function("disabled", |b| {
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

    // Benchmark with listener enabled
    {
        let provider = setup_provider_default();
        // Enable listener with RAII guard (after provider is built so tracepoints exist)
        let _guard = enable_listener_with_fallback(PROVIDER_NAME);
        let ot_layer = tracing_layer::OpenTelemetryTracingBridge::new(&provider);
        let subscriber = Registry::default().with(ot_layer);

        tracing::subscriber::with_default(subscriber, || {
            group.bench_function("enabled", |b| {
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

    group.finish();
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
