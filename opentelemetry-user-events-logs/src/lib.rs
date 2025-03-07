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
    use serde_json::{from_str, Value};
    use std::process::Command;
    use tracing::error;
    use tracing_subscriber::{layer::SubscriberExt, EnvFilter, Layer};

    #[test]
    fn integration_test_basic() {
        // Run using the below command
        // sudo -E ~/.cargo/bin/cargo test integration -- --nocapture
        check_user_events_available().expect("Kernel does not support user_events. Verify your distribution/kernel supports user_events: https://docs.kernel.org/trace/user_events.html.");

        let logger_provider = LoggerProviderBuilder::default()
            .with_user_event_exporter("myprovider")
            .build();

        let user_event_status = check_user_events_available().expect("Kernel does not support user_events. Verify your distribution/kernel supports user_events: https://docs.kernel.org/trace/user_events.html.");
        assert!(user_event_status.contains("myprovider_L1K1"));
        assert!(user_event_status.contains("myprovider_L2K1"));
        assert!(user_event_status.contains("myprovider_L3K1"));
        assert!(user_event_status.contains("myprovider_L4K1"));
        assert!(user_event_status.contains("myprovider_L5K1"));

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

        // Start perf recording in a separate thread
        let perf_thread =
            std::thread::spawn(|| run_perf_and_decode(5, "user_events:myprovider_L2K1"));

        // Give a little time for perf to start recording
        std::thread::sleep(std::time::Duration::from_millis(2000));

        // ACT
        error!(
            name: "my-event-name",
            target: "my-target",
            event_id = 20,
            user_name = "otel user",
            user_email = "otel.user@opentelemetry.com"
        );

        // Wait for the perf thread to complete and get the results
        let result = perf_thread.join().expect("Perf thread panicked");

        assert!(result.is_ok());
        let json_content = result.unwrap();
        assert!(!json_content.is_empty());

        println!("Decoded perf data: {}", json_content);

        /*
        Example output:
         { "n": "myprovider:my-event-name", "__csver__": 1024, "PartA": { "time": "2025-03-07T05:02:35.838954949+00:00" }, "PartC": { "user_name": "otel user", "user_email": "otel.user@opentelemetry.com" }, "PartB": { "_typeName": "Log", "severityNumber": 2, "severityText": "ERROR", "eventId": 20, "name": "my-event-name" }, "meta": { "time": 39919.929650336, "cpu": 7, "pid": 18411, "tid": 18412, "level": 2, "keyword": "0x1" } } ]}
         */

        // Additional assertions to verify the captured event contains the expected data
        // The JSON output might contain brackets at the beginning and end,
        // and might have multiple events. We need to handle both cases.
        let json_value: Value = if json_content.trim().starts_with('[') {
            // It's an array of events
            from_str(&json_content).expect("Failed to parse JSON array")
        } else {
            // It's a single event or needs to be wrapped
            from_str(&format!("[{}]", json_content)).expect("Failed to parse JSON object")
        };

        // Get the event from the array (even if there's only one)
        let events = json_value.as_array().expect("JSON is not an array");

        // Find our specific event - it should be the only one or at least one of them
        let event = events
            .iter()
            .find(|e| {
                if let Some(name) = e.get("n") {
                    name.as_str().unwrap_or("") == "myprovider:my-event-name"
                } else {
                    false
                }
            })
            .expect("Event 'myprovider:my-event-name' not found");

        // Validate event structure and fields
        assert_eq!(event["n"].as_str().unwrap(), "myprovider:my-event-name");
        assert_eq!(event["__csver__"].as_i64().unwrap(), 1024);

        // Validate PartA
        let part_a = &event["PartA"];
        assert!(part_a.get("time").is_some(), "PartA.time is missing");

        // Validate PartB
        let part_b = &event["PartB"];
        assert_eq!(part_b["_typeName"].as_str().unwrap(), "Log");
        assert_eq!(part_b["severityNumber"].as_i64().unwrap(), 2);
        assert_eq!(part_b["severityText"].as_str().unwrap(), "ERROR");
        assert_eq!(part_b["eventId"].as_i64().unwrap(), 20);
        assert_eq!(part_b["name"].as_str().unwrap(), "my-event-name");

        // Validate PartC
        let part_c = &event["PartC"];
        assert_eq!(part_c["user_name"].as_str().unwrap(), "otel user");
        assert_eq!(
            part_c["user_email"].as_str().unwrap(),
            "otel.user@opentelemetry.com"
        );
    }

    fn check_user_events_available() -> Result<String, String> {
        let output = Command::new("sudo")
            .arg("cat")
            .arg("/sys/kernel/tracing/user_events_status")
            .output()
            .map_err(|e| format!("Failed to execute command: {}", e))?;

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

    pub fn run_perf_and_decode(duration_secs: u64, event: &str) -> std::io::Result<String> {
        // Run perf record with timeout
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
