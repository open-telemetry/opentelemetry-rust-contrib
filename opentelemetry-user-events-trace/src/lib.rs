//! The user_events exporter will enable applications to use OpenTelemetry API
//! to capture the telemetry events, and write to user_events subsystem.

#![warn(missing_debug_implementations, missing_docs)]

mod trace;

pub use trace::*;

#[cfg(test)]
mod tests {

    use crate::UserEventsTracerProviderBuilderExt;
    use opentelemetry::trace::{Span, Tracer, TracerProvider};
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use serde_json::{from_str, Value};
    use std::process::Command;

    // Ignore as this cannot be run in Github CI due to lack of
    // required Kernel. Uncomment to run locally in a supported environment

    #[ignore]
    #[test]
    fn integration_test_basic() {
        // Run using the below command
        // sudo -E ~/.cargo/bin/cargo test integration_test_basic -- --nocapture --ignored

        // Basic check if user_events are available
        check_user_events_available().expect("Kernel does not support user_events. Verify your distribution/kernel supports user_events: https://docs.kernel.org/trace/user_events.html.");
        let provider = SdkTracerProvider::builder()
            .with_resource(
                opentelemetry_sdk::Resource::builder()
                    .with_service_name("myrolename")
                    .build(),
            )
            .with_user_events_exporter("opentelemetry_traces")
            .build();

        // Validate that the TracePoint is created.
        // There is no notion of Severity for Spans in OTel,
        // so we use the default severity level of 4 (Info).
        let user_event_status = check_user_events_available().expect("Kernel does not support user_events. Verify your distribution/kernel supports user_events: https://docs.kernel.org/trace/user_events.html.");
        assert!(user_event_status.contains("opentelemetry_traces_L4K1"));

        // Start perf recording in a separate thread and emit logs in parallel.
        let perf_thread =
            std::thread::spawn(|| run_perf_and_decode(5, "user_events:opentelemetry_traces_L4K1"));

        // Give a little time for perf to start recording
        std::thread::sleep(std::time::Duration::from_millis(1000));

        // ACT
        let tracer = provider.tracer("user-events-tracer");
        let mut span = tracer
            .span_builder("my-span-name")
            .with_attributes([opentelemetry::KeyValue::new("my-key", "my-value")])
            .start(&tracer);
        span.end();

        // Wait for the perf thread to complete and get the results
        let result = perf_thread.join().expect("Perf thread panicked");

        assert!(result.is_ok());
        let json_content = result.unwrap();
        assert!(!json_content.is_empty());

        let formatted_output = json_content.trim().to_string();
        /*
                // Sample output from perf-decode
        {
        "./perf.data": [
        { "n": "opentelemetry_traces:Span", "__csver__": 1024, "PartA": { "time": "2025-07-10T23:04:08.109839907+00:00", "ext_dt_traceId": "e8bbbe6db41c807792b93648ad9398e1", "ext_dt_spanId": "cfdb9dc3c3948453", "ext_cloud_role": "user-events-trace-example" }, "PartB": { "_typeName": "Span", "name": "my-span-name", "parentId": "", "startTime": "2025-07-10T23:04:08.109797282+00:00", "success": true, "kind": 0 }, "PartC": { "my-key": "my-value" }, "meta": { "time": 104590.077244551, "cpu": 7, "pid": 176542, "tid": 176542, "level": 4, "keyword": "0x1" } } ]
        }
                 */

        let json_value: Value = from_str(&formatted_output).expect("Failed to parse JSON");
        let perf_data_key = json_value
            .as_object()
            .expect("JSON is not an object")
            .keys()
            .find(|k| k.contains("perf.data"))
            .expect("No perf.data key found in JSON");

        let events = json_value[perf_data_key]
            .as_array()
            .expect("Events for perf.data is not an array");

        // Find the specific event. Its named providername:eventname format.
        let event = events
            .iter()
            .find(|e| {
                if let Some(name) = e.get("n") {
                    name.as_str().unwrap_or("") == "opentelemetry_traces:Span"
                } else {
                    false
                }
            })
            .expect("Event 'opentelemetry_traces:Span' not found");

        // Validate event structure and fields
        assert_eq!(event["n"].as_str().unwrap(), "opentelemetry_traces:Span");
        assert_eq!(event["__csver__"].as_i64().unwrap(), 1024);

        // Validate PartA
        let part_a = &event["PartA"];
        // Only check if the time field exists, not the actual value
        assert!(part_a.get("time").is_some(), "PartA.time is missing");

        let role = part_a
            .get("ext_cloud_role")
            .expect("PartA.ext_cloud_role is missing");
        assert_eq!(role.as_str().unwrap(), "myrolename");

        // Validate PartB
        let part_b = &event["PartB"];
        assert_eq!(part_b["_typeName"].as_str().unwrap(), "Span");
        assert_eq!(part_b["name"].as_str().unwrap(), "my-span-name");

        // Validate PartC
        let part_c = &event["PartC"];
        assert_eq!(part_c["my-key"].as_str().unwrap(), "my-value");
    }

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

    pub fn run_perf_and_decode(duration_secs: u64, event: &str) -> std::io::Result<String> {
        // Run perf record for duration_secs seconds
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

        // Change permissions on perf.data (which is the default file perf records to) to allow reading
        let chmod_status = Command::new("sudo")
            .args(["chmod", "uog+r", "./perf.data"])
            .status()?;

        if !chmod_status.success() {
            panic!("chmod failed with exit code: {:?}", chmod_status.code());
        }

        // Decode the performance data and return it directly
        // Note: This tool must be installed on the machine
        // git clone https://github.com/microsoft/LinuxTracepoints &&
        // cd LinuxTracepoints && mkdir build && cd build && cmake .. && make &&
        // sudo cp bin/perf-decode /usr/local/bin &&
        let decode_output = Command::new("perf-decode").args(["./perf.data"]).output()?;

        if !decode_output.status.success() {
            panic!(
                "perf-decode failed with exit code: {:?}",
                decode_output.status.code()
            );
        }

        // Convert the output to a String
        let raw_output = String::from_utf8_lossy(&decode_output.stdout).to_string();

        // Remove any Byte Order Mark (BOM) characters
        // UTF-8 BOM is EF BB BF (in hex)
        let cleaned_output = if let Some(stripped) = raw_output.strip_prefix('\u{FEFF}') {
            // Skip the BOM character
            stripped.to_string()
        } else {
            raw_output
        };

        // Trim the output to remove any leading/trailing whitespace
        let trimmed_output = cleaned_output.trim().to_string();

        Ok(trimmed_output)
    }
}
