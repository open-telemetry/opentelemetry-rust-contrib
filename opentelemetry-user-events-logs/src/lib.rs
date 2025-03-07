//! The user_events exporter will enable applications to use OpenTelemetry API
//! to capture the telemetry events, and write to user_events subsystem.

#![warn(missing_debug_implementations, missing_docs)]

mod logs;

pub use logs::*;

#[cfg(test)]
mod tests {

    use super::*;
    use opentelemetry_appender_tracing::layer;
    use opentelemetry_sdk::logs::LoggerProviderBuilder;
    use std::process::Command;
    use tracing::{error, info};
    use tracing_subscriber::{layer::SubscriberExt, EnvFilter, Layer};

    #[test]
    fn integration_test_basic() {
        // Run using
        // sudo -E ~/.cargo/bin/cargo test integration -- --nocapture
        check_user_events_available().expect("Kernel does not support user_events. Verify your distribution/kernel supports user_events: https://docs.kernel.org/trace/user_events.html.");

        let logger_provider = LoggerProviderBuilder::default()
            .with_user_event_exporter("myprovider")
            .build();

        let filter_otel =
            EnvFilter::new("info").add_directive("opentelemetry=off".parse().unwrap());
        let otel_layer = layer::OpenTelemetryTracingBridge::new(&logger_provider);
        let otel_layer = otel_layer.with_filter(filter_otel);

        let filter_fmt =
            EnvFilter::new("debug").add_directive("opentelemetry=debug".parse().unwrap());
        let fmt_layer = tracing_subscriber::fmt::layer().with_filter(filter_fmt);

        let subscriber = tracing_subscriber::registry()
            .with(otel_layer)
            .with(fmt_layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        // Give sometime for the tracepoints to be created
        // This is important because the tracepoints are created in the kernel
        // and it takes a little time for them to be available
        std::thread::sleep(std::time::Duration::from_millis(2000));

        // Start perf recording in a separate thread
        let perf_thread =
            std::thread::spawn(|| run_perf_and_decode(5, "user_events:myprovider_L2K1"));

        // Give a little time for perf to start recording
        std::thread::sleep(std::time::Duration::from_millis(2000));

        check_user_events_available().expect("Kernel does not support user_events. Verify your distribution/kernel supports user_events: https://docs.kernel.org/trace/user_events.html.");

        // Execute the code that should generate the events we want to capture
        println!("Generating event1");
        error!(
            name: "my-event-name",
            target: "my-target",
            event_id = 20,
            user_name = "otel user",
            user_email = "otel.user@opentelemtry.com"
        );

        std::thread::sleep(std::time::Duration::from_millis(500));
        println!("Generating event1");
        error!(
            name: "my-event-name",
            target: "my-target",
            event_id = 20,
            user_name = "otel user",
            user_email = "otel.user@opentelemtry.com"
        );

        println!("Generating events completed");

        // Add a small delay to ensure the event is captured
        std::thread::sleep(std::time::Duration::from_millis(500));

        // Wait for the perf thread to complete and get the results
        let result = perf_thread.join().expect("Perf thread panicked");

        assert!(result.is_ok());
        let json_content = result.unwrap();
        assert!(!json_content.is_empty());

        println!("Decoded perf data: {}", json_content);

        // Additional assertions to verify the captured event contains the expected data
        assert!(json_content.contains("my-event-name"));
        assert!(json_content.contains("event_id"));
        assert!(json_content.contains("otel user"));
    }

    fn check_user_events_available() -> Result<(), String> {
        let output = Command::new("sudo")
            .arg("cat")
            .arg("/sys/kernel/tracing/user_events_status")
            .output()
            .map_err(|e| format!("Failed to execute command: {}", e))?;

        if output.status.success() {
            let status = String::from_utf8_lossy(&output.stdout);
            info!(name = "UserEvent Status", "User events status: {}", status);
            Ok(())
        } else {
            Err(format!(
                "Command executed with failing error code: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }

    pub fn run_perf_and_decode(duration_secs: u64, event: &str) -> std::io::Result<String> {
        // Run perf record with timeout
        println!("Running perf record for {} seconds...", duration_secs);
        let perf_status = Command::new("sudo")
            .args([
                "timeout",
                "-s",
                "SIGINT",
                &duration_secs.to_string(),
                "perf",
                "record",
                "-e",
                event,
            ])
            .status()?;

        if !perf_status.success() {
            // Check if it's the expected signal termination (SIGINT from timeout)
            // timeout sends SIGINT, which will cause a non-zero exit code (130 typically)
            if !matches!(perf_status.code(), Some(124) | Some(130) | Some(143)) {
                panic!(
                    "perf record failed with exit code: {:?}",
                    perf_status.code()
                );
            }
        }

        println!("Perf record completed.");

        // Make the perf.data file world-readable
        let chmod_status = Command::new("sudo")
            .args(["chmod", "uog+r", "./perf.data"])
            .status()?;

        if !chmod_status.success() {
            panic!("chmod failed with exit code: {:?}", chmod_status.code());
        }

        // Decode the performance data and return it directly
        let decode_output = Command::new("perf-decode").args(["./perf.data"]).output()?;

        if !decode_output.status.success() {
            panic!(
                "perf-decode failed with exit code: {:?}",
                decode_output.status.code()
            );
        }

        // Convert the output to a String and return it
        Ok(String::from_utf8_lossy(&decode_output.stdout).to_string())
    }
}
