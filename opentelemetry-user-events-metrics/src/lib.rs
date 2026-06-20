mod exporter;
mod tracepoint;

pub use exporter::MetricsExporter;

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use crate::MetricsExporter;
    use opentelemetry::metrics::MeterProvider;
    use opentelemetry::KeyValue;
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use opentelemetry_sdk::Resource;

    mod test_utils {
        use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
        use prost::Message;

        use one_collect::perf_event::{RingBufBuilder, RingBufSessionBuilder};
        use one_collect::tracefs::TraceFS;
        use one_collect::Writable;

        /// Verifies that tracefs (and therefore user_events) is reachable. Returns
        /// a descriptive error if it is not.
        pub fn check_user_events_available() -> Result<(), String> {
            TraceFS::open().map(|_| ()).map_err(|e| {
                format!(
                    "Unable to open tracefs. user_events requires a Linux kernel \
                     with tracefs mounted and sufficient permissions \
                     (https://docs.kernel.org/trace/user_events.html): {e}"
                )
            })
        }

        /// Builds an in-process perf ring buffer session over the `otlp_metrics`
        /// user_events tracepoint, runs `emit` (which should record metrics and
        /// shut down the meter provider so the exporter writes its events into the
        /// now-enabled ring buffer), drains the ring buffer, and returns every
        /// decoded OTLP metrics payload.
        ///
        /// The `MetricsExporter` must already be created before calling this:
        /// creating the exporter registers the tracepoint, which is required for
        /// `find_event` to succeed.
        ///
        /// This replaces the previous `perf record` + `perf-decode` + JSON parsing
        /// pipeline with a self-contained, in-process consumer (no external tools,
        /// no temp files, no `sudo` shell-outs).
        pub fn collect_otlp_metrics<F: FnOnce()>(emit: F) -> Vec<ExportMetricsServiceRequest> {
            let need_permission = "Need permission to access tracefs/perf_events (run via sudo?)";

            let tracefs = TraceFS::open().expect(need_permission);
            let mut event = tracefs
                .find_event("user_events", "otlp_metrics")
                .expect("otlp_metrics tracepoint not found; create the MetricsExporter first");

            // The `buffer` field is declared as `__rel_loc u8[]` in the tracepoint
            // definition (see src/tracepoint/mod.rs). one_collect resolves the
            // rel_loc to the raw OTLP protobuf bytes for us.
            let buffer_ref = event.format().get_field_ref_unchecked("buffer");

            let collected = Writable::<Vec<ExportMetricsServiceRequest>>::new(Vec::new());
            let sink = collected.clone();

            event.add_callback(move |data| {
                let buffer = data.format().get_data(buffer_ref, data.event_data());
                match ExportMetricsServiceRequest::decode(buffer) {
                    Ok(request) => sink.write(|out| out.push(request)),
                    Err(e) => eprintln!("Failed to decode OTLP metrics from buffer: {e}"),
                }
                Ok(())
            });

            let mut session = RingBufSessionBuilder::new()
                .with_page_count(32)
                .with_tracepoint_events(RingBufBuilder::for_tracepoint())
                .with_target_pid(std::process::id() as i32)
                .build()
                .expect(need_permission);

            session
                .add_event(event)
                .expect("Failed to add otlp_metrics event to session");
            session.enable().expect(need_permission);

            // Record metrics and shut down the provider so the exporter writes its
            // events while the ring buffer is enabled and capturing.
            emit();

            // emit() shut the provider down synchronously, so every event is
            // already in the kernel ring buffer by the time we get here.
            // Disable the session first: this stops new collection but retains
            // the already-buffered records. Once disabled, `parse_all` drains
            // what's buffered and returns immediately (while a session is still
            // enabled, `parse_all` would keep polling and never return), so
            // there is no need for a timed wait.
            session.disable().expect(need_permission);
            session
                .parse_all()
                .expect("Failed to parse perf ring buffer");

            let mut decoded_metrics = Vec::new();
            collected.read(|v| decoded_metrics = v.clone());
            decoded_metrics
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

        // Collect the OTLP metrics emitted on provider shutdown by reading the
        // `otlp_metrics` user_events tracepoint directly from the perf ring buffer.
        let decoded_metrics = test_utils::collect_otlp_metrics(|| {
            provider
                .shutdown()
                .expect("Failed to shutdown meter provider");
        });

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
            "Should find data point with attributes: {expected_attributes_1:?}"
        );
        assert!(
            found_attributes_2,
            "Should find data point with attributes: {expected_attributes_2:?}"
        );

        println!("Success!");
    }

    #[ignore]
    #[test]
    fn integration_test_sync_gauge() {
        // sudo -E ~/.cargo/bin/cargo test integration_test_sync_gauge -- --nocapture --ignored

        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

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
        let gauge = meter
            .u64_gauge("gauge_u64_test")
            .with_description("sync gauge test")
            .with_unit("test_unit")
            .build();

        gauge.record(42, &[KeyValue::new("mykey1", "myvalue1")]);
        gauge.record(43, &[KeyValue::new("mykey1", "myvalueA")]);

        let decoded = test_utils::collect_otlp_metrics(|| {
            provider
                .shutdown()
                .expect("Failed to shutdown meter provider");
        });

        assert_eq!(
            decoded.len(),
            2,
            "Expected one event per data point (2 attribute sets)"
        );

        let mut values: Vec<(u64, Vec<KeyValue>)> = Vec::new();
        for req in &decoded {
            for rm in &req.resource_metrics {
                for sm in &rm.scope_metrics {
                    for m in &sm.metrics {
                        assert_eq!(m.name, "gauge_u64_test");
                        let data = m.data.as_ref().expect("metric data missing");
                        let dps = test_utils::extract_metric_data(data, 0);
                        assert_eq!(dps.len(), 1, "expected 1 data point per event");
                        let dp = &dps[0];
                        let value = match dp.value.as_ref().expect("value missing") {
                            opentelemetry_proto::tonic::metrics::v1::number_data_point::Value::AsInt(v) => *v as u64,
                            _ => panic!("expected integer value for u64 gauge"),
                        };
                        let mut attrs: Vec<KeyValue> = dp
                            .attributes
                            .iter()
                            .map(|a| {
                                let v = match a.value.as_ref().and_then(|v| v.value.as_ref()) {
                                    Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s)) => s.clone(),
                                    _ => panic!("unexpected attribute value type"),
                                };
                                KeyValue::new(a.key.clone(), v)
                            })
                            .collect();
                        attrs.sort_by(|a, b| a.key.as_str().cmp(b.key.as_str()));
                        values.push((value, attrs));
                    }
                }
            }
        }

        values.sort_by_key(|(v, _)| *v);
        assert_eq!(
            values,
            vec![
                (42, vec![KeyValue::new("mykey1", "myvalue1")]),
                (43, vec![KeyValue::new("mykey1", "myvalueA")]),
            ]
        );
    }

    #[ignore]
    #[test]
    fn integration_test_updowncounter() {
        // sudo -E ~/.cargo/bin/cargo test integration_test_updowncounter -- --nocapture --ignored

        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

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
        let udc = meter
            .i64_up_down_counter("updown_i64_test")
            .with_description("updowncounter test")
            .with_unit("test_unit")
            .build();

        // Net values per attribute set: set1 = 5, set2 = -3
        udc.add(10, &[KeyValue::new("mykey1", "myvalue1")]);
        udc.add(-5, &[KeyValue::new("mykey1", "myvalue1")]);
        udc.add(-3, &[KeyValue::new("mykey1", "myvalueA")]);

        let decoded = test_utils::collect_otlp_metrics(|| {
            provider
                .shutdown()
                .expect("Failed to shutdown meter provider");
        });

        assert_eq!(decoded.len(), 2, "Expected one event per attribute set");

        let mut results: Vec<(i64, Vec<KeyValue>, bool)> = Vec::new();
        for req in &decoded {
            for rm in &req.resource_metrics {
                for sm in &rm.scope_metrics {
                    for m in &sm.metrics {
                        assert_eq!(m.name, "updown_i64_test");
                        let data = m.data.as_ref().expect("metric data missing");
                        let sum = match data {
                            opentelemetry_proto::tonic::metrics::v1::metric::Data::Sum(s) => s,
                            _ => panic!("expected Sum data for updowncounter"),
                        };
                        assert!(!sum.is_monotonic, "updowncounter sum must be non-monotonic");
                        assert_eq!(sum.data_points.len(), 1);
                        let dp = &sum.data_points[0];
                        let value = match dp.value.as_ref().expect("value missing") {
                            opentelemetry_proto::tonic::metrics::v1::number_data_point::Value::AsInt(v) => *v,
                            _ => panic!("expected integer value for i64 updowncounter"),
                        };
                        let mut attrs: Vec<KeyValue> = dp
                            .attributes
                            .iter()
                            .map(|a| {
                                let v = match a.value.as_ref().and_then(|v| v.value.as_ref()) {
                                    Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s)) => s.clone(),
                                    _ => panic!("unexpected attribute value type"),
                                };
                                KeyValue::new(a.key.clone(), v)
                            })
                            .collect();
                        attrs.sort_by(|a, b| a.key.as_str().cmp(b.key.as_str()));
                        results.push((value, attrs, sum.is_monotonic));
                    }
                }
            }
        }

        results.sort_by_key(|(v, _, _)| *v);
        assert_eq!(
            results,
            vec![
                (-3, vec![KeyValue::new("mykey1", "myvalueA")], false),
                (5, vec![KeyValue::new("mykey1", "myvalue1")], false),
            ]
        );
    }

    #[ignore]
    #[test]
    fn integration_test_histogram() {
        // sudo -E ~/.cargo/bin/cargo test integration_test_histogram -- --nocapture --ignored

        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

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
        let hist = meter
            .f64_histogram("histogram_f64_test")
            .with_description("histogram test")
            .with_unit("test_unit")
            .build();

        let attrs = [KeyValue::new("mykey1", "myvalue1")];
        // Three observations: 1.0, 5.0, 10.0 → count=3, sum=16.0, min=1.0, max=10.0
        hist.record(1.0, &attrs);
        hist.record(5.0, &attrs);
        hist.record(10.0, &attrs);

        let decoded = test_utils::collect_otlp_metrics(|| {
            provider
                .shutdown()
                .expect("Failed to shutdown meter provider");
        });

        assert_eq!(
            decoded.len(),
            1,
            "Expected one event for the single attribute set"
        );

        let req = &decoded[0];
        let metric = &req.resource_metrics[0].scope_metrics[0].metrics[0];
        assert_eq!(metric.name, "histogram_f64_test");
        assert_eq!(metric.description, "histogram test");
        assert_eq!(metric.unit, "test_unit");

        let hist_data = match metric.data.as_ref().expect("metric data missing") {
            opentelemetry_proto::tonic::metrics::v1::metric::Data::Histogram(h) => h,
            _ => panic!("expected Histogram data"),
        };
        assert_eq!(hist_data.data_points.len(), 1);
        let dp = &hist_data.data_points[0];
        assert_eq!(dp.count, 3);
        assert_eq!(dp.sum, Some(16.0));
        assert_eq!(dp.min, Some(1.0));
        assert_eq!(dp.max, Some(10.0));
        // bucket_counts has one more entry than explicit_bounds
        assert_eq!(dp.bucket_counts.len(), dp.explicit_bounds.len() + 1);
        // Total of bucket counts must equal the data point count
        assert_eq!(dp.bucket_counts.iter().sum::<u64>(), dp.count);

        let mut actual_attrs: Vec<KeyValue> = dp
            .attributes
            .iter()
            .map(|a| {
                let v = match a.value.as_ref().and_then(|v| v.value.as_ref()) {
                    Some(
                        opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s),
                    ) => s.clone(),
                    _ => panic!("unexpected attribute value type"),
                };
                KeyValue::new(a.key.clone(), v)
            })
            .collect();
        actual_attrs.sort_by(|a, b| a.key.as_str().cmp(b.key.as_str()));
        assert_eq!(actual_attrs, vec![KeyValue::new("mykey1", "myvalue1")]);
    }
}
