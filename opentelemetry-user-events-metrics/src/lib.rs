mod exporter;
mod tracepoint;

pub use exporter::MetricsExporter;

#[cfg(test)]
mod tests {
    use crate::MetricsExporter;
    use opentelemetry::metrics::MeterProvider;
    use opentelemetry::KeyValue;
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use opentelemetry_sdk::Resource;

    mod test_utils {
        use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
        use prost::Message;
        use serde_json::{self, Value};
        use std::process::Command;

        /// Represents a user event record from perf data
        #[derive(Debug, Clone)]
        #[allow(dead_code)]
        pub struct UserEventRecord {
            pub name: String,
            pub protocol: u32,
            pub version: String,
            pub buffer: Vec<u8>,
        }

        /// Extract user event records from JSON content
        pub fn extract_user_events(
            json_content: &str,
        ) -> Result<Vec<UserEventRecord>, Box<dyn std::error::Error>> {
            let parsed: Value = serde_json::from_str(json_content)?;
            let mut records = Vec::new();

            // The JSON structure is { "./perf.data": [array of events] }
            if let Some(events_map) = parsed.as_object() {
                for (_, events_value) in events_map {
                    if let Some(events_array) = events_value.as_array() {
                        for event in events_array {
                            if let Some(record) = parse_user_event_record(event)? {
                                records.push(record);
                            }
                        }
                    }
                }
            }

            Ok(records)
        }

        /// Parse a single user event record from JSON (test-only)
        fn parse_user_event_record(
            event: &Value,
        ) -> Result<Option<UserEventRecord>, Box<dyn std::error::Error>> {
            let name = event["n"].as_str().unwrap_or("").to_string();
            let protocol = event["protocol"].as_u64().unwrap_or(0) as u32;
            let version = event["version"].as_str().unwrap_or("").to_string();

            // Extract buffer as Vec<u8>
            let buffer = if let Some(buffer_array) = event["buffer"].as_array() {
                buffer_array
                    .iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u8))
                    .collect()
            } else {
                Vec::new()
            };

            Ok(Some(UserEventRecord {
                name,
                protocol,
                version,
                buffer,
            }))
        }

        /// Decode OTLP protobuf buffer to ExportMetricsServiceRequest (test-only)
        fn decode_otlp_metrics(
            buffer: &[u8],
        ) -> Result<ExportMetricsServiceRequest, Box<dyn std::error::Error>> {
            let request = ExportMetricsServiceRequest::decode(buffer)?;
            Ok(request)
        }

        /// Helper function to process all OTLP metrics from JSON content (test-only)
        pub fn extract_and_decode_otlp_metrics(
            json_content: &str,
        ) -> Result<Vec<ExportMetricsServiceRequest>, Box<dyn std::error::Error>> {
            let user_events = extract_user_events(json_content)?;
            let mut decoded_metrics = Vec::new();

            for event in user_events {
                // Filter for OTLP metrics events
                if event.name.contains("otlp_metrics") {
                    match decode_otlp_metrics(&event.buffer) {
                        Ok(metrics_request) => {
                            decoded_metrics.push(metrics_request);
                        }
                        Err(e) => {
                            eprintln!("Failed to decode OTLP metrics from buffer: {}", e);
                            // Continue processing other events instead of failing completely
                        }
                    }
                }
            }

            Ok(decoded_metrics)
        }

        pub fn check_user_events_available() -> Result<String, String> {
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

        /// Extract metric data from different metric types
        /// Returns a reference to the data points vector for the given metric type
        /// TODO: Add support for more metric types like Histogram and ExponentialHistogram
        /// This function assumes that the metric data is either Sum or Gauge type
        pub fn extract_metric_data(
            metric_data: &opentelemetry_proto::tonic::metrics::v1::metric::Data,
            request_index: usize,
        ) -> &Vec<opentelemetry_proto::tonic::metrics::v1::NumberDataPoint> {
            match metric_data {
                opentelemetry_proto::tonic::metrics::v1::metric::Data::Sum(sum) => &sum.data_points,
                opentelemetry_proto::tonic::metrics::v1::metric::Data::Gauge(gauge) => {
                    &gauge.data_points
                }
                // TODO: Add support for Histogram and ExponentialHistogram
                // These will need special handling as they don't use NumberDataPoint:
                // opentelemetry_proto::tonic::metrics::v1::metric::Data::Histogram(hist) => {
                //     // Histogram uses HistogramDataPoint instead of NumberDataPoint
                //     // Will need separate handling or abstraction
                // }
                // opentelemetry_proto::tonic::metrics::v1::metric::Data::ExponentialHistogram(exp_hist) => {
                //     // ExponentialHistogram uses ExponentialHistogramDataPoint
                //     // Will need separate handling or abstraction
                // }
                _ => panic!(
                    "Unsupported metric data type in request {}",
                    request_index + 1
                ),
            }
        }

        /// Extract and validate data point from metric data (supports Sum and Gauge types)
        /// Returns the attributes from the single data point after validation
        pub fn extract_and_validate_metric_data(
            metric: &opentelemetry_proto::tonic::metrics::v1::Metric,
            expected_value: u64,
            request_index: usize,
        ) -> Vec<opentelemetry::KeyValue> {
            if let Some(data) = &metric.data {
                // Use helper method to extract data points based on metric type
                let data_points = extract_metric_data(data, request_index);

                // Validate exactly one data point
                assert_eq!(
                    data_points.len(),
                    1,
                    "Request {} should have exactly one data point",
                    request_index + 1
                );

                let data_point = &data_points[0];

                // Validate counter value
                if let Some(value) = &data_point.value {
                    match value {
                        opentelemetry_proto::tonic::metrics::v1::number_data_point::Value::AsInt(int_val) => {
                            assert_eq!(*int_val as u64, expected_value,
                                "Counter value should match expected value in request {}", request_index + 1);
                        }
                        _ => panic!("Expected integer value for u64 counter in request {}", request_index + 1),
                    }
                }

                // Extract attributes from data point
                let mut actual_attributes: Vec<opentelemetry::KeyValue> = Vec::new();
                for attr in &data_point.attributes {
                    if let Some(value) = &attr.value {
                        if let Some(string_value) = &value.value {
                            match string_value {
                                opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s) => {
                                    actual_attributes.push(opentelemetry::KeyValue::new(attr.key.clone(), s.clone()));
                                }
                                _ => {
                                    panic!("Unsupported attribute value type for key: {} in request {}", attr.key, request_index + 1);
                                }
                            }
                        }
                    }
                }

                // Sort attributes for consistent comparison
                actual_attributes.sort_by(|a, b| a.key.as_str().cmp(b.key.as_str()));
                actual_attributes
            } else {
                panic!("Metric data is missing in request {}", request_index + 1);
            }
        }
    }

    #[ignore]
    #[test]
    fn integration_test_basic() {
        // Run using the below command
        // sudo -E ~/.cargo/bin/cargo test integration_test_basic -- --nocapture --ignored

        test_utils::check_user_events_available().expect("Kernel does not support user_events. Verify your distribution/kernel supports user_events: https://docs.kernel.org/trace/user_events.html.");

        let exporter = MetricsExporter::new();
        let provider = SdkMeterProvider::builder()
            .with_resource(
                Resource::builder_empty()
                    .with_attributes(vec![KeyValue::new("service.name", "metric-demo")])
                    .build(),
            )
            .with_periodic_exporter(exporter)
            .build();

        let meter = provider.meter("user-event-test");

        // Create a Counter Instrument.
        let counter = meter
            .u64_counter("counter_u64_test")
            .with_description("test_decription")
            .with_unit("test_unit")
            .build();

        counter.add(
            1,
            &[
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ],
        );

        counter.add(
            1,
            &[
                KeyValue::new("mykey1", "myvalueA"),
                KeyValue::new("mykey2", "myvalueB"),
            ],
        );

        let perf_thread = std::thread::spawn(move || {
            test_utils::run_perf_and_decode(5, "user_events:otlp_metrics".as_ref())
        });

        // Give a little time for perf to start recording
        std::thread::sleep(std::time::Duration::from_millis(1000));

        provider
            .shutdown()
            .expect("Failed to shutdown meter provider");
        let result = perf_thread.join().expect("Perf thread panicked");

        assert!(result.is_ok());
        let json_content = result.unwrap();
        assert!(!json_content.is_empty());

        let formatted_output = json_content.trim().to_string();
        println!("Formatted Output: {}", formatted_output);

        // Extract and decode OTLP metrics from the JSON content
        let decoded_metrics = test_utils::extract_and_decode_otlp_metrics(&formatted_output)
            .expect("Failed to extract and decode OTLP metrics");

        // Expected values from the test setup
        let expected_counter_name = "counter_u64_test";
        let expected_description = "test_decription";
        let expected_unit = "test_unit";
        let expected_value = 1u64;
        // Create expected attributes in sorted order (by key)
        let expected_attributes_1 = vec![
            KeyValue::new("mykey1", "myvalue1"),
            KeyValue::new("mykey2", "myvalue2"),
        ];
        let expected_attributes_2 = vec![
            KeyValue::new("mykey1", "myvalueA"),
            KeyValue::new("mykey2", "myvalueB"),
        ];
        let expected_service_name = "metric-demo";
        let expected_meter_name = "user-event-test";

        // STEP 1: Validate upfront that we have exactly 2 entries
        assert_eq!(
            decoded_metrics.len(),
            2,
            "Expected exactly 2 metrics payloads (one per data point)"
        );

        // STEP 2: Do common validation on both entries (resource, scope, metric metadata)
        for (index, metrics_request) in decoded_metrics.iter().enumerate() {
            println!(
                "Validating common elements for Metrics Request {}",
                index + 1
            );

            // Validate resource metrics structure
            assert!(
                !metrics_request.resource_metrics.is_empty(),
                "Metrics request {} should have resource metrics",
                index + 1
            );

            for resource_metric in &metrics_request.resource_metrics {
                // Validate resource attributes (service.name)
                if let Some(resource) = &resource_metric.resource {
                    let service_name_attr = resource
                        .attributes
                        .iter()
                        .find(|attr| attr.key == "service.name");
                    if let Some(attr) = service_name_attr {
                        if let Some(value) = &attr.value {
                            if let Some(string_value) = &value.value {
                                match string_value {
                                    opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s) => {
                                        assert_eq!(s, expected_service_name,
                                            "Service name should match expected value in request {}", index + 1);
                                    }
                                    _ => panic!("Service name attribute should be a string value in request {}", index + 1),
                                }
                            }
                        }
                    }
                }

                for scope_metric in &resource_metric.scope_metrics {
                    // Validate scope/meter name
                    if let Some(scope) = &scope_metric.scope {
                        assert_eq!(
                            scope.name,
                            expected_meter_name,
                            "Meter name should match expected value in request {}",
                            index + 1
                        );
                    }

                    // Validate metrics metadata (should be consistent across both requests)
                    for metric in &scope_metric.metrics {
                        if metric.name == expected_counter_name {
                            assert_eq!(
                                metric.name,
                                expected_counter_name,
                                "Metric name should match expected value in request {}",
                                index + 1
                            );
                            assert_eq!(
                                metric.description,
                                expected_description,
                                "Metric description should match expected value in request {}",
                                index + 1
                            );
                            assert_eq!(
                                metric.unit,
                                expected_unit,
                                "Metric unit should match expected value in request {}",
                                index + 1
                            );
                        }
                    }
                }
            }
        }

        // STEP 3: Validate that each entry has exactly one data point and collect attributes
        let mut actual_attribute_sets = Vec::new();

        for (index, metrics_request) in decoded_metrics.iter().enumerate() {
            println!("Validating data points for Metrics Request {}", index + 1);

            for resource_metric in &metrics_request.resource_metrics {
                for scope_metric in &resource_metric.scope_metrics {
                    for metric in &scope_metric.metrics {
                        if metric.name == expected_counter_name {
                            // Use helper method to extract and validate metric data
                            let actual_attributes = test_utils::extract_and_validate_metric_data(
                                metric,
                                expected_value,
                                index,
                            );
                            actual_attribute_sets.push(actual_attributes);
                        }
                    }
                }
            }
        }

        // STEP 4: Validate that both expected attribute sets are present (order independent)
        assert_eq!(
            actual_attribute_sets.len(),
            2,
            "Should have collected exactly 2 data points"
        );

        // Check that both expected attribute sets are present (order independent)
        // Note: expected_attributes are already in sorted order by key
        let mut found_attributes_1 = false;
        let mut found_attributes_2 = false;

        for actual_attributes in &actual_attribute_sets {
            if actual_attributes == &expected_attributes_1 {
                found_attributes_1 = true;
            } else if actual_attributes == &expected_attributes_2 {
                found_attributes_2 = true;
            }
        }

        assert!(
            found_attributes_1,
            "Should find data point with attributes: {:?}",
            expected_attributes_1
        );
        assert!(
            found_attributes_2,
            "Should find data point with attributes: {:?}",
            expected_attributes_2
        );

        println!("Success!");
    }
}
