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
            collect_otlp_metrics_with_pages(32, emit)
        }

        /// Same as [`collect_otlp_metrics`], but with a configurable per-CPU ring
        /// buffer size. High-cardinality scenarios emit several hundred kilobytes
        /// in a single export cycle and will silently lose records if the ring
        /// buffer is left at the default 32 pages (128 KiB).
        pub fn collect_otlp_metrics_with_pages<F: FnOnce()>(
            page_count: usize,
            emit: F,
        ) -> Vec<ExportMetricsServiceRequest> {
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
            // A captured event that does not decode is a wire-format failure and
            // must fail the test. Recording it here rather than panicking keeps
            // the panic out of the perf callback, where it would unwind through
            // the parsing library.
            let decode_errors = Writable::<Vec<String>>::new(Vec::new());
            let error_sink = decode_errors.clone();

            event.add_callback(move |data| {
                let buffer = data.format().get_data(buffer_ref, data.event_data());
                match ExportMetricsServiceRequest::decode(buffer) {
                    Ok(request) => sink.write(|out| out.push(request)),
                    Err(e) => error_sink.write(|out| {
                        out.push(format!("{} byte event failed to decode: {e}", buffer.len()))
                    }),
                }
                Ok(())
            });

            let mut session = RingBufSessionBuilder::new()
                .with_page_count(page_count)
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

            let mut errors = Vec::new();
            decode_errors.read(|v| errors = v.clone());
            assert!(
                errors.is_empty(),
                "the exporter wrote {} event(s) that are not valid OTLP: {errors:#?}",
                errors.len()
            );

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

        /// Extracts the sorted attribute set of every data point in `metric`,
        /// validating each value against `expected_value`.
        ///
        /// The exporter packs as many data points as fit into one event, so a
        /// single payload legitimately carries several data points.
        pub fn extract_and_validate_metric_data(
            metric: &opentelemetry_proto::tonic::metrics::v1::Metric,
            expected_value: u64,
            request_index: usize,
        ) -> Vec<Vec<opentelemetry::KeyValue>> {
            let Some(data) = &metric.data else {
                panic!("Metric data is missing in request {}", request_index + 1);
            };

            let data_points = extract_metric_data(data, request_index);
            assert!(
                !data_points.is_empty(),
                "Request {} should carry at least one data point",
                request_index + 1
            );

            data_points
                .iter()
                .map(|data_point| {
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
                })
                .collect()
        }

        /// A decoded numeric data point value.
        #[derive(Debug, Clone, Copy, PartialEq)]
        pub enum Num {
            I(i64),
            D(f64),
        }

        /// Renders an OTLP `AnyValue` into a stable string form so tests can
        /// assert on attribute values of any type without a match arm per type.
        pub fn render_value(value: &opentelemetry_proto::tonic::common::v1::AnyValue) -> String {
            use opentelemetry_proto::tonic::common::v1::any_value::Value;
            match value.value.as_ref() {
                Some(Value::StringValue(s)) => s.clone(),
                Some(Value::BoolValue(b)) => b.to_string(),
                Some(Value::IntValue(i)) => i.to_string(),
                Some(Value::DoubleValue(d)) => d.to_string(),
                Some(Value::ArrayValue(a)) => {
                    let rendered: Vec<String> = a.values.iter().map(render_value).collect();
                    format!("[{}]", rendered.join(","))
                }
                Some(Value::BytesValue(b)) => format!("{b:?}"),
                Some(Value::KvlistValue(_)) => "<kvlist>".to_string(),
                Some(other) => format!("{other:?}"),
                None => "<empty>".to_string(),
            }
        }

        /// Extracts a data point's attributes as sorted `(key, rendered value)`
        /// pairs.
        pub fn attrs_of(
            attributes: &[opentelemetry_proto::tonic::common::v1::KeyValue],
        ) -> Vec<(String, String)> {
            let mut out: Vec<(String, String)> = attributes
                .iter()
                .map(|a| {
                    let rendered = a.value.as_ref().map(render_value).unwrap_or_default();
                    (a.key.clone(), rendered)
                })
                .collect();
            out.sort();
            out
        }

        /// Returns every occurrence of `name` across all events. A metric appears
        /// once per event it was batched into, so this legitimately returns more
        /// than one entry for a high-cardinality metric.
        pub fn find_metrics<'a>(
            requests: &'a [ExportMetricsServiceRequest],
            name: &str,
        ) -> Vec<&'a opentelemetry_proto::tonic::metrics::v1::Metric> {
            requests
                .iter()
                .flat_map(|r| &r.resource_metrics)
                .flat_map(|rm| &rm.scope_metrics)
                .flat_map(|sm| &sm.metrics)
                .filter(|m| m.name == name)
                .collect()
        }

        /// Flattens every `NumberDataPoint` of `name` across all events into
        /// `(sorted attributes, value)` pairs.
        pub fn number_points(
            requests: &[ExportMetricsServiceRequest],
            name: &str,
        ) -> Vec<(Vec<(String, String)>, Num)> {
            use opentelemetry_proto::tonic::metrics::v1::metric::Data;
            use opentelemetry_proto::tonic::metrics::v1::number_data_point::Value;

            let mut out = Vec::new();
            for metric in find_metrics(requests, name) {
                let points = match metric.data.as_ref().expect("metric data missing") {
                    Data::Sum(s) => &s.data_points,
                    Data::Gauge(g) => &g.data_points,
                    other => panic!("metric {name} is not a Sum or Gauge: {other:?}"),
                };
                for dp in points {
                    let value = match dp.value.as_ref().expect("data point value missing") {
                        Value::AsInt(i) => Num::I(*i),
                        Value::AsDouble(d) => Num::D(*d),
                    };
                    out.push((attrs_of(&dp.attributes), value));
                }
            }
            out
        }

        /// Flattens every `HistogramDataPoint` of `name` across all events.
        pub fn histogram_points(
            requests: &[ExportMetricsServiceRequest],
            name: &str,
        ) -> Vec<(
            Vec<(String, String)>,
            opentelemetry_proto::tonic::metrics::v1::HistogramDataPoint,
        )> {
            use opentelemetry_proto::tonic::metrics::v1::metric::Data;

            let mut out = Vec::new();
            for metric in find_metrics(requests, name) {
                let Data::Histogram(h) = metric.data.as_ref().expect("metric data missing") else {
                    panic!("metric {name} is not a Histogram");
                };
                for dp in &h.data_points {
                    out.push((attrs_of(&dp.attributes), dp.clone()));
                }
            }
            out
        }

        /// Asserts that no event exceeds the exporter's per-event size budget.
        ///
        /// This is the invariant the whole batching scheme rests on. A record
        /// that exceeds `PERF_MAX_TRACE_SIZE` is refused by
        /// `perf_trace_buf_alloc()` and never submitted, while the write still
        /// reports success, so an event that overshoots is silently lost rather
        /// than merely large. Because these payloads came back out of the
        /// kernel, this also proves the bound end to end.
        pub fn assert_all_events_within_size_limit(requests: &[ExportMetricsServiceRequest]) {
            for (index, request) in requests.iter().enumerate() {
                let len = request.encoded_len();
                assert!(
                    len <= crate::exporter::MAX_EVENT_SIZE,
                    "event {} is {} bytes, over the {} byte limit",
                    index,
                    len,
                    crate::exporter::MAX_EVENT_SIZE
                );
            }
        }

        /// Flattens every `ExponentialHistogramDataPoint` of `name` across all
        /// events.
        pub fn exp_histogram_points(
            requests: &[ExportMetricsServiceRequest],
            name: &str,
        ) -> Vec<(
            Vec<(String, String)>,
            opentelemetry_proto::tonic::metrics::v1::ExponentialHistogramDataPoint,
        )> {
            use opentelemetry_proto::tonic::metrics::v1::metric::Data;

            let mut out = Vec::new();
            for metric in find_metrics(requests, name) {
                let Data::ExponentialHistogram(h) =
                    metric.data.as_ref().expect("metric data missing")
                else {
                    panic!("metric {name} is not an ExponentialHistogram");
                };
                for dp in &h.data_points {
                    out.push((attrs_of(&dp.attributes), dp.clone()));
                }
            }
            out
        }

        /// Asserts that every event carries exactly one scope holding exactly one
        /// metric.
        ///
        /// The exporter reuses a single `ExportMetricsServiceRequest` across
        /// metrics and scopes, mutating it in place. If a metric's data were ever
        /// left behind when the next metric is processed, a consumer would see
        /// duplicated or misattributed data points. This is the assertion that
        /// would catch that.
        pub fn assert_one_metric_per_event(requests: &[ExportMetricsServiceRequest]) {
            for (index, request) in requests.iter().enumerate() {
                assert_eq!(
                    request.resource_metrics.len(),
                    1,
                    "event {index} should carry exactly one resource_metrics"
                );
                let scope_metrics = &request.resource_metrics[0].scope_metrics;
                assert_eq!(
                    scope_metrics.len(),
                    1,
                    "event {index} should carry exactly one scope_metrics"
                );
                assert_eq!(
                    scope_metrics[0].metrics.len(),
                    1,
                    "event {index} should carry exactly one metric, got {:?}",
                    scope_metrics[0]
                        .metrics
                        .iter()
                        .map(|m| m.name.clone())
                        .collect::<Vec<_>>()
                );
            }
        }

        /// Returns the scope name attached to each event.
        pub fn scope_names(requests: &[ExportMetricsServiceRequest]) -> Vec<String> {
            requests
                .iter()
                .flat_map(|r| &r.resource_metrics)
                .flat_map(|rm| &rm.scope_metrics)
                .map(|sm| sm.scope.as_ref().expect("scope missing").name.clone())
                .collect()
        }

        /// Asserts that every event carrying `name` repeats its identifying
        /// metadata. Batching must not emit a bare continuation event that a
        /// consumer could not interpret on its own.
        pub fn assert_metric_metadata_repeated(
            requests: &[ExportMetricsServiceRequest],
            name: &str,
            expected_description: &str,
            expected_unit: &str,
        ) {
            let metrics = find_metrics(requests, name);
            assert!(!metrics.is_empty(), "metric {name} was never exported");
            for (index, metric) in metrics.iter().enumerate() {
                assert_eq!(
                    metric.description, expected_description,
                    "occurrence {index} of {name} lost its description"
                );
                assert_eq!(
                    metric.unit, expected_unit,
                    "occurrence {index} of {name} lost its unit"
                );
            }
        }

        /// Asserts that every event repeats the full resource and scope envelope.
        ///
        /// Batching amortizes the envelope across the data points inside one
        /// event, but each event must remain independently decodable, so the
        /// envelope must still be present in all of them.
        pub fn assert_envelope_repeated(
            requests: &[ExportMetricsServiceRequest],
            expected_resource_attrs: &[(&str, &str)],
            expected_scope_name: &str,
        ) {
            assert!(!requests.is_empty(), "expected at least one event");
            for (index, request) in requests.iter().enumerate() {
                assert_eq!(
                    request.resource_metrics.len(),
                    1,
                    "event {index} should carry exactly one resource_metrics"
                );
                let rm = &request.resource_metrics[0];
                let resource = rm.resource.as_ref().expect("resource missing");
                let actual = attrs_of(&resource.attributes);
                for (key, value) in expected_resource_attrs {
                    assert!(
                        actual.contains(&((*key).to_string(), (*value).to_string())),
                        "event {index} is missing resource attribute {key}={value}, got {actual:?}"
                    );
                }
                assert_eq!(
                    rm.scope_metrics.len(),
                    1,
                    "event {index} should carry exactly one scope_metrics"
                );
                let scope = rm.scope_metrics[0].scope.as_ref().expect("scope missing");
                assert_eq!(
                    scope.name, expected_scope_name,
                    "event {index} has the wrong scope name"
                );
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

        // STEP 1: Both data points are small and share one metric, so the
        // exporter must pack them into a single event.
        assert_eq!(
            decoded_metrics.len(),
            1,
            "Expected a single batched payload carrying both data points"
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

        // STEP 3: Collect the attribute set of every data point across all events
        let mut actual_attribute_sets = Vec::new();

        for (index, metrics_request) in decoded_metrics.iter().enumerate() {
            println!("Validating data points for Metrics Request {}", index + 1);

            for resource_metric in &metrics_request.resource_metrics {
                for scope_metric in &resource_metric.scope_metrics {
                    for metric in &scope_metric.metrics {
                        if metric.name == expected_counter_name {
                            // Use helper method to extract and validate metric data
                            actual_attribute_sets.extend(
                                test_utils::extract_and_validate_metric_data(
                                    metric,
                                    expected_value,
                                    index,
                                ),
                            );
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
            1,
            "Expected both attribute sets to be packed into a single event"
        );

        let mut values: Vec<(u64, Vec<KeyValue>)> = Vec::new();
        for req in &decoded {
            for rm in &req.resource_metrics {
                for sm in &rm.scope_metrics {
                    for m in &sm.metrics {
                        assert_eq!(m.name, "gauge_u64_test");
                        let data = m.data.as_ref().expect("metric data missing");
                        let dps = test_utils::extract_metric_data(data, 0);
                        for dp in dps {
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

        assert_eq!(
            decoded.len(),
            1,
            "Expected both attribute sets to be packed into a single event"
        );

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
                        for dp in &sum.data_points {
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

    /// Builds a provider whose resource carries a few attributes, mirroring a
    /// realistic (small) Overlake-style resource.
    fn test_provider() -> SdkMeterProvider {
        SdkMeterProvider::builder()
            .with_resource(
                Resource::builder_empty()
                    .with_attributes(vec![
                        KeyValue::new("service.name", "metric-demo"),
                        KeyValue::new("service.namespace", "demo-ns"),
                        KeyValue::new("host.name", "test-host"),
                    ])
                    .build(),
            )
            .with_periodic_exporter(MetricsExporter::new())
            .build()
    }

    const RESOURCE_ATTRS: &[(&str, &str)] = &[
        ("service.name", "metric-demo"),
        ("service.namespace", "demo-ns"),
        ("host.name", "test-host"),
    ];

    /// High-cardinality end-to-end batching test.
    ///
    /// This is the test that actually proves the batching change against the
    /// kernel rather than against a mock: 2000 distinct series are exported,
    /// read back out of the perf ring buffer, and checked for exact
    /// preservation. Because every data point must be accounted for, it also
    /// proves that events packed up to `MAX_EVENT_SIZE` are actually delivered
    /// rather than silently refused by `perf_trace_buf_alloc()`, which is the
    /// reason `MAX_EVENT_SIZE` exists at all.
    #[ignore]
    #[test]
    fn integration_test_batching_high_cardinality() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        const SERIES: usize = 2000;

        let provider = test_provider();
        let meter = provider.meter("user-event-test");
        let counter = meter.u64_counter("counter_high_cardinality").build();

        for i in 0..SERIES {
            counter.add(
                1,
                &[
                    KeyValue::new("partition", format!("p{i}")),
                    KeyValue::new("region", "westus2"),
                    KeyValue::new("cluster", "cluster-a"),
                ],
            );
        }

        // 2000 series is a few hundred KiB; the default 32-page ring buffer
        // would drop records and make this test flaky.
        let decoded = test_utils::collect_otlp_metrics_with_pages(1024, || {
            provider
                .shutdown()
                .expect("Failed to shutdown meter provider");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        test_utils::assert_envelope_repeated(&decoded, RESOURCE_ATTRS, "user-event-test");

        // Batching must collapse many data points into far fewer events. With
        // ~60 byte data points and an 8168 byte budget this is roughly a 100x
        // reduction; assert a very loose 10x so the test is about the behaviour,
        // not about a particular encoding size.
        assert!(
            decoded.len() * 10 < SERIES,
            "expected batching to produce far fewer than {} events, got {}",
            SERIES,
            decoded.len()
        );

        let points = test_utils::number_points(&decoded, "counter_high_cardinality");
        assert_eq!(
            points.len(),
            SERIES,
            "every data point must be exported exactly once"
        );

        let mut partitions: Vec<String> = points
            .iter()
            .map(|(attrs, value)| {
                assert_eq!(*value, test_utils::Num::I(1), "unexpected counter value");
                assert!(
                    attrs.contains(&("region".to_string(), "westus2".to_string())),
                    "data point lost its constant attributes: {attrs:?}"
                );
                attrs
                    .iter()
                    .find(|(k, _)| k == "partition")
                    .map(|(_, v)| v.clone())
                    .expect("partition attribute missing")
            })
            .collect();
        partitions.sort();
        partitions.dedup();
        assert_eq!(
            partitions.len(),
            SERIES,
            "data points were duplicated or dropped"
        );
    }

    /// Every event except the last must be packed until the next data point no
    /// longer fits. This guards against a regression that silently flushes early
    /// and gives back the byte savings.
    #[ignore]
    #[test]
    fn integration_test_batching_packs_events_to_capacity() {
        use opentelemetry_proto::tonic::metrics::v1::metric::Data;
        use prost::Message;

        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        let provider = test_provider();
        let meter = provider.meter("user-event-test");
        let counter = meter.u64_counter("counter_packing").build();

        // Stay at or below the SDK's default cardinality limit (2000 series per
        // instrument). Beyond it the SDK folds the excess into a single
        // `otel.metric.overflow` data point, which would make the count below
        // ambiguous: a shortfall could mean either aggregation overflow or an
        // event lost in the kernel, and this test needs to detect only the
        // latter. 2000 series still fills many events at MAX_EVENT_SIZE.
        const SERIES: usize = 2000;

        for i in 0..SERIES {
            counter.add(1, &[KeyValue::new("partition", format!("p{i:05}"))]);
        }

        let decoded = test_utils::collect_otlp_metrics_with_pages(1024, || {
            provider
                .shutdown()
                .expect("Failed to shutdown meter provider");
        });

        assert!(
            decoded.len() > 1,
            "test needs enough data points to fill more than one event, got {} events",
            decoded.len()
        );
        test_utils::assert_all_events_within_size_limit(&decoded);

        // Packing density is only meaningful if nothing was lost on the way.
        // An event packed a few bytes over the real kernel budget is refused by
        // `perf_trace_buf_alloc()` without any error surfacing to the writer, so
        // without this check a `MAX_EVENT_SIZE` that is slightly too large would
        // still satisfy every assertion below.
        let points = test_utils::number_points(&decoded, "counter_packing");
        assert_eq!(
            points.len(),
            SERIES,
            "every data point must survive the round trip through the kernel"
        );

        // The most expensive data point anywhere in the export. If an event has
        // at least this much room left over, the batcher could have fitted
        // another point into it and flushed too early.
        let point_cost = |dp: &opentelemetry_proto::tonic::metrics::v1::NumberDataPoint| {
            let len = dp.encoded_len();
            // field tag (1 byte, field number 1) + length delimiter + payload
            1 + prost::length_delimiter_len(len) + len
        };
        let max_point_cost = decoded
            .iter()
            .flat_map(|r| &r.resource_metrics)
            .flat_map(|rm| &rm.scope_metrics)
            .flat_map(|sm| &sm.metrics)
            .map(|m| {
                let Data::Sum(sum) = m.data.as_ref().expect("metric data missing") else {
                    panic!("expected Sum data");
                };
                sum.data_points.iter().map(point_cost).max().unwrap_or(0)
            })
            .max()
            .expect("no data points were exported");

        // Records from a single export can land in different per-CPU ring
        // buffers if the exporter thread migrates, so the order they are read
        // back in is not guaranteed. Assert an order-independent property
        // instead: only the final (partial) event may be under-filled.
        let underfilled = decoded
            .iter()
            .filter(|request| {
                crate::exporter::MAX_EVENT_SIZE - request.encoded_len()
                    >= max_point_cost + crate::exporter::SIZE_SLACK
            })
            .count();
        assert!(
            underfilled <= 1,
            "{} of {} events had room for another data point; only the final partial event may be \
             under-filled (largest data point costs {} bytes)",
            underfilled,
            decoded.len(),
            max_point_cost
        );
    }

    /// A single data point too large to ever fit in one event is dropped, and
    /// crucially the surrounding data points are still exported.
    #[ignore]
    #[test]
    fn integration_test_oversized_data_point_is_dropped_but_others_survive() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        let provider = test_provider();
        let meter = provider.meter("user-event-test");
        let counter = meter.u64_counter("counter_oversized").build();

        counter.add(1, &[KeyValue::new("partition", "small-a")]);
        // Comfortably larger than MAX_EVENT_SIZE on its own.
        counter.add(1, &[KeyValue::new("partition", "X".repeat(70_000))]);
        counter.add(1, &[KeyValue::new("partition", "small-b")]);

        let decoded = test_utils::collect_otlp_metrics_with_pages(1024, || {
            // The dropped data point surfaces as an export error, which is the
            // documented behaviour; the test asserts on what was exported.
            let _ = provider.shutdown();
        });

        test_utils::assert_all_events_within_size_limit(&decoded);

        let points = test_utils::number_points(&decoded, "counter_oversized");
        let partitions: Vec<String> = points
            .iter()
            .map(|(attrs, _)| {
                attrs
                    .iter()
                    .find(|(k, _)| k == "partition")
                    .map(|(_, v)| v.clone())
                    .expect("partition attribute missing")
            })
            .collect();

        assert_eq!(
            partitions.len(),
            2,
            "expected exactly the two small data points to survive, got {partitions:?}"
        );
        let mut sorted = partitions.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec!["small-a".to_string(), "small-b".to_string()],
            "the oversized data point must be dropped and the two small ones emitted \
             exactly once each: {partitions:?}"
        );
        assert!(
            points
                .iter()
                .all(|(_, value)| *value == test_utils::Num::I(1)),
            "surviving data point values were altered: {points:?}"
        );
    }

    /// Batching is per-metric, so three instruments in one export cycle produce
    /// three independent events, each carrying its own full envelope.
    #[ignore]
    #[test]
    fn integration_test_multiple_metrics_in_one_cycle() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        let provider = test_provider();
        let meter = provider.meter("user-event-test");

        meter
            .u64_counter("multi_counter")
            .build()
            .add(7, &[KeyValue::new("k", "v")]);
        meter
            .u64_gauge("multi_gauge")
            .build()
            .record(11, &[KeyValue::new("k", "v")]);
        meter
            .f64_histogram("multi_histogram")
            .build()
            .record(2.5, &[KeyValue::new("k", "v")]);

        let decoded = test_utils::collect_otlp_metrics(|| {
            provider
                .shutdown()
                .expect("Failed to shutdown meter provider");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        test_utils::assert_envelope_repeated(&decoded, RESOURCE_ATTRS, "user-event-test");

        assert_eq!(
            decoded.len(),
            3,
            "expected one event per metric, got {}",
            decoded.len()
        );
        for request in &decoded {
            assert_eq!(
                request.resource_metrics[0].scope_metrics[0].metrics.len(),
                1,
                "each event should carry exactly one metric"
            );
        }

        assert_eq!(
            test_utils::number_points(&decoded, "multi_counter"),
            vec![(
                vec![("k".to_string(), "v".to_string())],
                test_utils::Num::I(7)
            )]
        );
        assert_eq!(
            test_utils::number_points(&decoded, "multi_gauge"),
            vec![(
                vec![("k".to_string(), "v".to_string())],
                test_utils::Num::I(11)
            )]
        );
        let hist = test_utils::histogram_points(&decoded, "multi_histogram");
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].1.count, 1);
        assert_eq!(hist[0].1.sum, Some(2.5));
    }

    /// Each instrumentation scope must be emitted in its own event with its own
    /// scope metadata (name, version, schema URL).
    #[ignore]
    #[test]
    fn integration_test_multiple_meters_keep_scope_metadata() {
        use opentelemetry::InstrumentationScope;

        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        let provider = test_provider();

        let meter_a = provider.meter_with_scope(
            InstrumentationScope::builder("scope.a")
                .with_version("1.2.3")
                .with_schema_url("https://example.com/schema/a")
                .build(),
        );
        let meter_b = provider.meter_with_scope(
            InstrumentationScope::builder("scope.b")
                .with_version("4.5.6")
                .build(),
        );

        meter_a
            .u64_counter("scoped_counter_a")
            .build()
            .add(1, &[KeyValue::new("k", "v")]);
        meter_b
            .u64_counter("scoped_counter_b")
            .build()
            .add(2, &[KeyValue::new("k", "v")]);

        let decoded = test_utils::collect_otlp_metrics(|| {
            provider
                .shutdown()
                .expect("Failed to shutdown meter provider");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        assert_eq!(decoded.len(), 2, "expected one event per scope");

        let mut scopes: Vec<(String, String, String)> = decoded
            .iter()
            .map(|r| {
                let sm = &r.resource_metrics[0].scope_metrics[0];
                let scope = sm.scope.as_ref().expect("scope missing");
                (
                    scope.name.clone(),
                    scope.version.clone(),
                    sm.schema_url.clone(),
                )
            })
            .collect();
        scopes.sort();

        assert_eq!(
            scopes,
            vec![
                (
                    "scope.a".to_string(),
                    "1.2.3".to_string(),
                    "https://example.com/schema/a".to_string()
                ),
                ("scope.b".to_string(), "4.5.6".to_string(), String::new()),
            ]
        );

        assert_eq!(
            test_utils::number_points(&decoded, "scoped_counter_a")[0].1,
            test_utils::Num::I(1)
        );
        assert_eq!(
            test_utils::number_points(&decoded, "scoped_counter_b")[0].1,
            test_utils::Num::I(2)
        );
    }

    /// Asynchronous instruments go through the same batching path as synchronous
    /// ones, and must preserve monotonicity and aggregation temporality.
    #[ignore]
    #[test]
    fn integration_test_observable_instruments() {
        use opentelemetry_proto::tonic::metrics::v1::metric::Data;
        use opentelemetry_proto::tonic::metrics::v1::AggregationTemporality;

        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        let provider = test_provider();
        let meter = provider.meter("user-event-test");

        let _obs_counter = meter
            .u64_observable_counter("obs_counter")
            .with_callback(|o| {
                o.observe(100, &[KeyValue::new("k", "a")]);
                o.observe(200, &[KeyValue::new("k", "b")]);
            })
            .build();
        let _obs_udc = meter
            .i64_observable_up_down_counter("obs_updowncounter")
            .with_callback(|o| o.observe(-5, &[KeyValue::new("k", "a")]))
            .build();
        let _obs_gauge = meter
            .u64_observable_gauge("obs_gauge")
            .with_callback(|o| o.observe(42, &[KeyValue::new("k", "a")]))
            .build();

        let decoded = test_utils::collect_otlp_metrics(|| {
            provider
                .shutdown()
                .expect("Failed to shutdown meter provider");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        test_utils::assert_envelope_repeated(&decoded, RESOURCE_ATTRS, "user-event-test");

        let mut counter_points = test_utils::number_points(&decoded, "obs_counter");
        counter_points.sort_by_key(|(attrs, _)| attrs.clone());
        assert_eq!(
            counter_points,
            vec![
                (
                    vec![("k".to_string(), "a".to_string())],
                    test_utils::Num::I(100)
                ),
                (
                    vec![("k".to_string(), "b".to_string())],
                    test_utils::Num::I(200)
                ),
            ]
        );

        assert_eq!(
            test_utils::number_points(&decoded, "obs_updowncounter"),
            vec![(
                vec![("k".to_string(), "a".to_string())],
                test_utils::Num::I(-5)
            )]
        );
        assert_eq!(
            test_utils::number_points(&decoded, "obs_gauge"),
            vec![(
                vec![("k".to_string(), "a".to_string())],
                test_utils::Num::I(42)
            )]
        );

        // Monotonicity and temporality must survive batching.
        for metric in test_utils::find_metrics(&decoded, "obs_counter") {
            let Data::Sum(sum) = metric.data.as_ref().unwrap() else {
                panic!("obs_counter should be a Sum");
            };
            assert!(sum.is_monotonic, "observable counter must be monotonic");
            assert_eq!(
                sum.aggregation_temporality,
                AggregationTemporality::Delta as i32,
                "exporter declares Delta temporality"
            );
        }
        for metric in test_utils::find_metrics(&decoded, "obs_updowncounter") {
            let Data::Sum(sum) = metric.data.as_ref().unwrap() else {
                panic!("obs_updowncounter should be a Sum");
            };
            assert!(
                !sum.is_monotonic,
                "observable updowncounter must be non-monotonic"
            );
        }
        for metric in test_utils::find_metrics(&decoded, "obs_gauge") {
            assert!(
                matches!(metric.data.as_ref().unwrap(), Data::Gauge(_)),
                "obs_gauge should be a Gauge"
            );
        }
    }

    /// Floating point instruments must round-trip as `AsDouble`, not be coerced
    /// to integers.
    #[ignore]
    #[test]
    fn integration_test_f64_instruments() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        let provider = test_provider();
        let meter = provider.meter("user-event-test");

        let counter = meter.f64_counter("counter_f64").build();
        counter.add(1.5, &[KeyValue::new("k", "a")]);
        counter.add(2.25, &[KeyValue::new("k", "a")]);

        let udc = meter.f64_up_down_counter("updown_f64").build();
        udc.add(-0.5, &[KeyValue::new("k", "a")]);

        let decoded = test_utils::collect_otlp_metrics(|| {
            provider
                .shutdown()
                .expect("Failed to shutdown meter provider");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        assert_eq!(
            test_utils::number_points(&decoded, "counter_f64"),
            vec![(
                vec![("k".to_string(), "a".to_string())],
                test_utils::Num::D(3.75)
            )]
        );
        assert_eq!(
            test_utils::number_points(&decoded, "updown_f64"),
            vec![(
                vec![("k".to_string(), "a".to_string())],
                test_utils::Num::D(-0.5)
            )]
        );
    }

    /// Attribute values of every supported type must survive encoding. The
    /// batching path re-encodes data points individually, so this guards against
    /// a type being lost or coerced during that step.
    #[ignore]
    #[test]
    fn integration_test_attribute_value_types() {
        use opentelemetry::{Array, StringValue, Value};

        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        let provider = test_provider();
        let meter = provider.meter("user-event-test");
        let counter = meter.u64_counter("counter_attr_types").build();

        counter.add(
            1,
            &[
                KeyValue::new("str", "text"),
                KeyValue::new("bool", true),
                KeyValue::new("int", 42i64),
                KeyValue::new("double", 1.5f64),
                KeyValue::new(
                    "str_array",
                    Value::Array(Array::String(vec![
                        StringValue::from("a"),
                        StringValue::from("b"),
                    ])),
                ),
            ],
        );

        let decoded = test_utils::collect_otlp_metrics(|| {
            provider
                .shutdown()
                .expect("Failed to shutdown meter provider");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        let points = test_utils::number_points(&decoded, "counter_attr_types");
        assert_eq!(points.len(), 1);

        let expected = vec![
            ("bool".to_string(), "true".to_string()),
            ("double".to_string(), "1.5".to_string()),
            ("int".to_string(), "42".to_string()),
            ("str".to_string(), "text".to_string()),
            ("str_array".to_string(), "[a,b]".to_string()),
        ];
        assert_eq!(points[0].0, expected);
    }

    /// A data point with no attributes at all must still be exported.
    #[ignore]
    #[test]
    fn integration_test_no_attributes() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        let provider = test_provider();
        let meter = provider.meter("user-event-test");
        meter.u64_counter("counter_no_attrs").build().add(9, &[]);

        let decoded = test_utils::collect_otlp_metrics(|| {
            provider
                .shutdown()
                .expect("Failed to shutdown meter provider");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        assert_eq!(
            test_utils::number_points(&decoded, "counter_no_attrs"),
            vec![(Vec::new(), test_utils::Num::I(9))]
        );
    }

    /// `force_flush` must export the current cycle, and because the exporter
    /// declares Delta temporality a subsequent cycle must carry only what was
    /// recorded since the previous export.
    #[ignore]
    #[test]
    fn integration_test_force_flush_across_cycles_is_delta() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        let provider = test_provider();
        let meter = provider.meter("user-event-test");
        let counter = meter.u64_counter("counter_delta").build();

        counter.add(3, &[KeyValue::new("k", "a")]);
        let first = test_utils::collect_otlp_metrics(|| {
            provider.force_flush().expect("first force_flush failed");
        });
        assert_eq!(
            test_utils::number_points(&first, "counter_delta"),
            vec![(
                vec![("k".to_string(), "a".to_string())],
                test_utils::Num::I(3)
            )]
        );

        counter.add(4, &[KeyValue::new("k", "a")]);
        let second = test_utils::collect_otlp_metrics(|| {
            provider.force_flush().expect("second force_flush failed");
        });
        assert_eq!(
            test_utils::number_points(&second, "counter_delta"),
            vec![(
                vec![("k".to_string(), "a".to_string())],
                test_utils::Num::I(4)
            )],
            "Delta temporality means the second cycle reports only the increment"
        );

        provider.shutdown().expect("shutdown failed");
    }

    /// An export cycle with nothing recorded must not emit any event.
    #[ignore]
    #[test]
    fn integration_test_no_metrics_emits_no_events() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        let provider = test_provider();
        let _meter = provider.meter("user-event-test");

        let decoded = test_utils::collect_otlp_metrics(|| {
            provider
                .shutdown()
                .expect("Failed to shutdown meter provider");
        });

        assert!(
            decoded.is_empty(),
            "expected no events when nothing was recorded, got {}",
            decoded.len()
        );
    }

    /// Exponential histograms use a different data point type than every other
    /// instrument, so they exercise a distinct arm of the batching code.
    #[ignore]
    #[test]
    fn integration_test_exponential_histogram() {
        use opentelemetry_proto::tonic::metrics::v1::metric::Data;
        use opentelemetry_sdk::metrics::{Aggregation, Instrument, Stream};

        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        let view = |i: &Instrument| {
            if i.name() == "exp_histogram" {
                Some(
                    Stream::builder()
                        .with_aggregation(Aggregation::Base2ExponentialHistogram {
                            max_size: 160,
                            max_scale: 20,
                            record_min_max: true,
                        })
                        .build()
                        .unwrap(),
                )
            } else {
                None
            }
        };

        let provider = SdkMeterProvider::builder()
            .with_resource(
                Resource::builder_empty()
                    .with_attributes(vec![
                        KeyValue::new("service.name", "metric-demo"),
                        KeyValue::new("service.namespace", "demo-ns"),
                        KeyValue::new("host.name", "test-host"),
                    ])
                    .build(),
            )
            .with_periodic_exporter(MetricsExporter::new())
            .with_view(view)
            .build();

        let meter = provider.meter("user-event-test");
        let hist = meter.f64_histogram("exp_histogram").build();
        for attr in ["a", "b"] {
            let attrs = [KeyValue::new("k", attr)];
            hist.record(1.0, &attrs);
            hist.record(4.0, &attrs);
            hist.record(16.0, &attrs);
        }

        let decoded = test_utils::collect_otlp_metrics(|| {
            provider
                .shutdown()
                .expect("Failed to shutdown meter provider");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        test_utils::assert_envelope_repeated(&decoded, RESOURCE_ATTRS, "user-event-test");
        assert_eq!(
            decoded.len(),
            1,
            "both attribute sets should be packed into one event"
        );

        let metrics = test_utils::find_metrics(&decoded, "exp_histogram");
        assert_eq!(metrics.len(), 1);
        let Data::ExponentialHistogram(exp) = metrics[0].data.as_ref().unwrap() else {
            panic!("expected ExponentialHistogram data");
        };
        assert_eq!(
            exp.data_points.len(),
            2,
            "both attribute sets must be batched into the same event"
        );
        for dp in &exp.data_points {
            assert_eq!(dp.count, 3);
            assert_eq!(dp.sum, Some(21.0));
            assert_eq!(dp.min, Some(1.0));
            assert_eq!(dp.max, Some(16.0));
        }
    }

    /// Histogram data points are much larger than number data points, so this
    /// checks that batching stays correct (and within the size limit) for the
    /// data point type most likely to overflow an event.
    #[ignore]
    #[test]
    fn integration_test_histogram_batching_many_attribute_sets() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        const SERIES: usize = 500;

        let provider = test_provider();
        let meter = provider.meter("user-event-test");
        let hist = meter.f64_histogram("histogram_batched").build();

        for i in 0..SERIES {
            let attrs = [KeyValue::new("partition", format!("p{i:04}"))];
            hist.record(1.0, &attrs);
            hist.record(3.0, &attrs);
        }

        let decoded = test_utils::collect_otlp_metrics_with_pages(1024, || {
            provider
                .shutdown()
                .expect("Failed to shutdown meter provider");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        test_utils::assert_envelope_repeated(&decoded, RESOURCE_ATTRS, "user-event-test");
        assert!(
            decoded.len() > 1,
            "histogram data points are large enough that 500 series should span several events"
        );

        let points = test_utils::histogram_points(&decoded, "histogram_batched");
        assert_eq!(
            points.len(),
            SERIES,
            "every histogram data point must survive"
        );

        let mut partitions: Vec<String> = points
            .iter()
            .map(|(attrs, dp)| {
                assert_eq!(dp.count, 2);
                assert_eq!(dp.sum, Some(4.0));
                assert_eq!(dp.min, Some(1.0));
                assert_eq!(dp.max, Some(3.0));
                assert_eq!(
                    dp.bucket_counts.len(),
                    dp.explicit_bounds.len() + 1,
                    "bucket layout must survive batching"
                );
                assert_eq!(dp.bucket_counts.iter().sum::<u64>(), dp.count);
                attrs
                    .iter()
                    .find(|(k, _)| k == "partition")
                    .map(|(_, v)| v.clone())
                    .expect("partition attribute missing")
            })
            .collect();
        partitions.sort();
        partitions.dedup();
        assert_eq!(
            partitions.len(),
            SERIES,
            "histogram data points were duplicated or dropped"
        );
    }

    // ---------------------------------------------------------------------
    // Batching across events, per instrument type.
    //
    // The batching code has a separate arm for each data point type. Sums and
    // histograms are covered above; these cover the remaining arms and, more
    // importantly, assert that the type-specific metadata (temporality,
    // monotonicity, bucket layout) is repeated on every event a metric is split
    // across, not just the first.
    // ---------------------------------------------------------------------

    /// An observable gauge with enough distinct attribute sets to span several
    /// events. Gauges carry no temporality, so the invariant here is simply that
    /// every point survives and every event is independently decodable.
    #[ignore]
    #[test]
    fn integration_test_gauge_batching_splits_across_events() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        const SERIES: usize = 800;

        let provider = test_provider();
        let meter = provider.meter("user-event-test");
        meter
            .u64_observable_gauge("gauge_batched")
            .with_callback(|obs| {
                for i in 0..SERIES {
                    obs.observe(
                        i as u64,
                        &[
                            KeyValue::new("partition", format!("p{i:05}")),
                            KeyValue::new("region", "westus2"),
                        ],
                    );
                }
            })
            .build();

        let decoded = test_utils::collect_otlp_metrics_with_pages(1024, || {
            provider.shutdown().expect("shutdown failed");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        test_utils::assert_envelope_repeated(&decoded, RESOURCE_ATTRS, "user-event-test");
        test_utils::assert_one_metric_per_event(&decoded);
        assert!(
            decoded.len() > 1,
            "expected the gauge to span several events, got {}",
            decoded.len()
        );

        let points = test_utils::number_points(&decoded, "gauge_batched");
        assert_eq!(points.len(), SERIES, "every gauge point must be exported");

        let mut seen: Vec<(String, test_utils::Num)> = points
            .iter()
            .map(|(attrs, value)| {
                let partition = attrs
                    .iter()
                    .find(|(k, _)| k == "partition")
                    .map(|(_, v)| v.clone())
                    .expect("partition attribute missing");
                (partition, *value)
            })
            .collect();
        seen.sort_by(|a, b| a.0.cmp(&b.0));
        seen.dedup_by(|a, b| a.0 == b.0);
        assert_eq!(
            seen.len(),
            SERIES,
            "gauge points were duplicated or dropped"
        );

        for (partition, value) in seen {
            let index: usize = partition
                .trim_start_matches('p')
                .parse()
                .expect("partition should be p<number>");
            assert_eq!(
                value,
                test_utils::Num::I(index as i64),
                "gauge point {partition} carries the wrong value"
            );
        }
    }

    /// A non-monotonic sum split across events. `is_monotonic` and the
    /// temporality live on the `Sum` message, which is rebuilt for every event,
    /// so a batching bug could easily produce a first event that is correct and
    /// later events that are not.
    #[ignore]
    #[test]
    fn integration_test_updowncounter_batching_preserves_sum_flags() {
        use opentelemetry_proto::tonic::metrics::v1::metric::Data;
        use opentelemetry_proto::tonic::metrics::v1::AggregationTemporality;

        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        const SERIES: usize = 600;

        let provider = test_provider();
        let meter = provider.meter("user-event-test");
        let udc = meter.i64_up_down_counter("updown_batched").build();
        for i in 0..SERIES {
            // Alternate sign so the values cannot be confused with a counter.
            let delta = if i % 2 == 0 { 5 } else { -5 };
            udc.add(delta, &[KeyValue::new("partition", format!("p{i:05}"))]);
        }

        let decoded = test_utils::collect_otlp_metrics_with_pages(1024, || {
            provider.shutdown().expect("shutdown failed");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        test_utils::assert_one_metric_per_event(&decoded);
        assert!(
            decoded.len() > 1,
            "expected the up/down counter to span several events, got {}",
            decoded.len()
        );

        let metrics = test_utils::find_metrics(&decoded, "updown_batched");
        assert_eq!(
            metrics.len(),
            decoded.len(),
            "every event should carry the metric"
        );
        for (index, metric) in metrics.iter().enumerate() {
            let Data::Sum(sum) = metric.data.as_ref().expect("metric data missing") else {
                panic!("occurrence {index} is not a Sum");
            };
            assert!(
                !sum.is_monotonic,
                "occurrence {index} lost is_monotonic=false; a consumer would treat \
                 an up/down counter as a monotonic counter"
            );
            assert_eq!(
                sum.aggregation_temporality,
                AggregationTemporality::Cumulative as i32,
                "occurrence {index} has the wrong temporality; the SDK aggregates \
                 up/down counters cumulatively"
            );
        }

        let points = test_utils::number_points(&decoded, "updown_batched");
        assert_eq!(points.len(), SERIES, "every data point must be exported");
        let negatives = points
            .iter()
            .filter(|(_, v)| *v == test_utils::Num::I(-5))
            .count();
        assert_eq!(
            negatives,
            SERIES / 2,
            "negative deltas must survive batching intact"
        );
    }

    /// Exponential histogram data points are the largest and most structured of
    /// the four types, with nested positive/negative bucket messages. This packs
    /// enough of them to span several events and verifies the nested structure
    /// survives in each.
    #[ignore]
    #[test]
    fn integration_test_exponential_histogram_batching_splits_across_events() {
        use opentelemetry_sdk::metrics::{Aggregation, Instrument, Stream};

        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        const SERIES: usize = 300;

        let view = |i: &Instrument| {
            if i.name() == "exp_histogram_batched" {
                Some(
                    Stream::builder()
                        .with_aggregation(Aggregation::Base2ExponentialHistogram {
                            max_size: 160,
                            max_scale: 20,
                            record_min_max: true,
                        })
                        .build()
                        .unwrap(),
                )
            } else {
                None
            }
        };

        let provider = SdkMeterProvider::builder()
            .with_resource(
                Resource::builder_empty()
                    .with_attributes(vec![
                        KeyValue::new("service.name", "metric-demo"),
                        KeyValue::new("service.namespace", "demo-ns"),
                        KeyValue::new("host.name", "test-host"),
                    ])
                    .build(),
            )
            .with_periodic_exporter(MetricsExporter::new())
            .with_view(view)
            .build();

        let meter = provider.meter("user-event-test");
        let hist = meter.f64_histogram("exp_histogram_batched").build();
        for i in 0..SERIES {
            let attrs = [KeyValue::new("partition", format!("p{i:05}"))];
            // A spread of magnitudes so the buckets are genuinely populated,
            // including a negative value and an exact zero.
            hist.record(0.0, &attrs);
            hist.record(1.5, &attrs);
            hist.record(64.0, &attrs);
            hist.record(-2.0, &attrs);
        }

        let decoded = test_utils::collect_otlp_metrics_with_pages(1024, || {
            provider.shutdown().expect("shutdown failed");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        test_utils::assert_envelope_repeated(&decoded, RESOURCE_ATTRS, "user-event-test");
        test_utils::assert_one_metric_per_event(&decoded);
        assert!(
            decoded.len() > 1,
            "expected the exponential histogram to span several events, got {}",
            decoded.len()
        );

        let points = test_utils::exp_histogram_points(&decoded, "exp_histogram_batched");
        assert_eq!(points.len(), SERIES, "every data point must be exported");

        let mut partitions = Vec::new();
        for (attrs, dp) in &points {
            assert_eq!(dp.count, 4, "each series recorded four measurements");
            assert_eq!(
                dp.zero_count, 1,
                "the 0.0 measurement must land in zero_count"
            );
            assert!(
                dp.positive.is_some() && dp.negative.is_some(),
                "both bucket sets must be present after batching"
            );
            let negative = dp.negative.as_ref().unwrap();
            assert_eq!(
                negative.bucket_counts.iter().sum::<u64>(),
                1,
                "the -2.0 measurement must land in a negative bucket"
            );
            assert_eq!(dp.min, Some(-2.0), "min must survive batching");
            assert_eq!(dp.max, Some(64.0), "max must survive batching");
            partitions.push(
                attrs
                    .iter()
                    .find(|(k, _)| k == "partition")
                    .map(|(_, v)| v.clone())
                    .expect("partition attribute missing"),
            );
        }
        partitions.sort();
        partitions.dedup();
        assert_eq!(
            partitions.len(),
            SERIES,
            "exponential histogram points were duplicated or dropped"
        );
    }

    /// Identifying metadata must be repeated on every event a metric is split
    /// across. An event that arrived without it would be undecodable on its own,
    /// and since events can be reordered or individually lost, a consumer cannot
    /// recover it from a neighbour.
    #[ignore]
    #[test]
    fn integration_test_metric_metadata_repeated_on_every_split_event() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        const SERIES: usize = 1500;

        let provider = test_provider();
        let meter = provider.meter("user-event-test");
        let counter = meter
            .u64_counter("counter_with_metadata")
            .with_description("requests handled by the ingest tier")
            .with_unit("{request}")
            .build();
        for i in 0..SERIES {
            counter.add(1, &[KeyValue::new("partition", format!("p{i:05}"))]);
        }

        let decoded = test_utils::collect_otlp_metrics_with_pages(1024, || {
            provider.shutdown().expect("shutdown failed");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        assert!(
            decoded.len() > 1,
            "expected the metric to span several events, got {}",
            decoded.len()
        );
        test_utils::assert_metric_metadata_repeated(
            &decoded,
            "counter_with_metadata",
            "requests handled by the ingest tier",
            "{request}",
        );
        assert_eq!(
            test_utils::number_points(&decoded, "counter_with_metadata").len(),
            SERIES
        );
    }

    /// Several metrics, each large enough to split, exported in one cycle.
    ///
    /// The exporter mutates one `ExportMetricsServiceRequest` in place as it
    /// walks metrics, so this is the scenario where state left over from one
    /// metric could leak into the next.
    #[ignore]
    #[test]
    fn integration_test_multiple_metrics_each_split_without_cross_contamination() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        const SERIES: usize = 700;
        const NAMES: [&str; 3] = ["counter_alpha", "counter_beta", "counter_gamma"];

        let provider = test_provider();
        let meter = provider.meter("user-event-test");
        for (index, name) in NAMES.iter().enumerate() {
            let counter = meter.u64_counter(*name).build();
            for i in 0..SERIES {
                // A distinct value per metric makes misattribution detectable.
                counter.add(
                    index as u64 + 1,
                    &[KeyValue::new("partition", format!("{name}-p{i:05}"))],
                );
            }
        }

        let decoded = test_utils::collect_otlp_metrics_with_pages(2048, || {
            provider.shutdown().expect("shutdown failed");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        test_utils::assert_one_metric_per_event(&decoded);

        for (index, name) in NAMES.iter().enumerate() {
            let points = test_utils::number_points(&decoded, name);
            assert_eq!(
                points.len(),
                SERIES,
                "{name} lost or duplicated data points"
            );
            for (attrs, value) in &points {
                assert_eq!(
                    *value,
                    test_utils::Num::I(index as i64 + 1),
                    "{name} carries a value belonging to another metric"
                );
                let partition = attrs
                    .iter()
                    .find(|(k, _)| k == "partition")
                    .map(|(_, v)| v.clone())
                    .expect("partition attribute missing");
                assert!(
                    partition.starts_with(name),
                    "{name} carries a data point belonging to another metric: {partition}"
                );
            }
        }
    }

    /// Several scopes, each with a metric large enough to split. Scope identity
    /// is part of the envelope rebuilt for every event, so this checks that an
    /// event is never attributed to the wrong meter.
    #[ignore]
    #[test]
    fn integration_test_multiple_scopes_each_split() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        const SERIES: usize = 600;
        const SCOPES: [&str; 3] = ["scope-alpha", "scope-beta", "scope-gamma"];

        let provider = test_provider();
        for scope in SCOPES {
            let meter = provider.meter(scope);
            let counter = meter.u64_counter(format!("counter_{scope}")).build();
            for i in 0..SERIES {
                counter.add(1, &[KeyValue::new("partition", format!("p{i:05}"))]);
            }
        }

        let decoded = test_utils::collect_otlp_metrics_with_pages(2048, || {
            provider.shutdown().expect("shutdown failed");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        test_utils::assert_one_metric_per_event(&decoded);

        // Every event must pair the right scope with the right metric.
        for request in &decoded {
            let sm = &request.resource_metrics[0].scope_metrics[0];
            let scope_name = sm.scope.as_ref().expect("scope missing").name.clone();
            let metric_name = sm.metrics[0].name.clone();
            assert_eq!(
                metric_name,
                format!("counter_{scope_name}"),
                "metric {metric_name} was attributed to scope {scope_name}"
            );
        }

        for scope in SCOPES {
            let points = test_utils::number_points(&decoded, &format!("counter_{scope}"));
            assert_eq!(points.len(), SERIES, "{scope} lost or duplicated points");
        }

        let names = test_utils::scope_names(&decoded);
        for scope in SCOPES {
            assert!(
                names.iter().any(|n| n == scope),
                "no event was emitted for scope {scope}"
            );
        }
    }

    // ---------------------------------------------------------------------
    // Size boundary behaviour.
    // ---------------------------------------------------------------------

    /// Sweeps the per-data-point size so that batches land on many different
    /// alignments relative to `MAX_EVENT_SIZE`.
    ///
    /// The batching loop decides whether the next point fits using an
    /// incrementally accumulated length plus `SIZE_SLACK`. An off-by-one there
    /// would only show up at particular sizes, which a single fixed-size test
    /// would very likely miss.
    #[ignore]
    #[test]
    fn integration_test_batching_boundary_size_sweep() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        // Sizes chosen to straddle protobuf varint length boundaries (127/128
        // and 16383/16384 are where the length delimiter grows).
        const VALUE_SIZES: [usize; 10] = [1, 63, 126, 127, 128, 129, 255, 512, 1024, 4096];
        const SERIES: usize = 40;

        for size in VALUE_SIZES {
            let provider = test_provider();
            let meter = provider.meter("user-event-test");
            let counter = meter.u64_counter("counter_sweep").build();
            for i in 0..SERIES {
                // The index keeps the series distinct; the padding sets the size.
                let value = format!("{i:05}{}", "x".repeat(size));
                counter.add(1, &[KeyValue::new("partition", value)]);
            }

            let decoded = test_utils::collect_otlp_metrics_with_pages(1024, || {
                provider.shutdown().expect("shutdown failed");
            });

            test_utils::assert_all_events_within_size_limit(&decoded);
            test_utils::assert_one_metric_per_event(&decoded);

            let points = test_utils::number_points(&decoded, "counter_sweep");
            assert_eq!(
                points.len(),
                SERIES,
                "value size {size}: expected {SERIES} data points, got {} across {} events",
                points.len(),
                decoded.len()
            );

            let mut partitions: Vec<String> = points
                .iter()
                .map(|(attrs, _)| {
                    attrs
                        .iter()
                        .find(|(k, _)| k == "partition")
                        .map(|(_, v)| v.clone())
                        .expect("partition attribute missing")
                })
                .collect();
            partitions.sort();
            partitions.dedup();
            assert_eq!(
                partitions.len(),
                SERIES,
                "value size {size}: data points were duplicated"
            );
        }
    }

    /// Walks a single data point up to and past the per-event limit.
    ///
    /// Delivery must be monotonic in size: once a point is too large it must
    /// stay too large. A non-monotonic result would mean the size accounting
    /// disagrees with what the kernel accepts, which is the failure mode that
    /// loses data silently in production.
    #[ignore]
    #[test]
    fn integration_test_single_data_point_size_limit_is_monotonic() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        // Around the 8168 byte budget, minus the resource/scope/metric envelope.
        const SIZES: [usize; 9] = [4096, 6144, 7168, 7680, 7900, 8000, 8100, 8192, 16384];

        let mut delivered = Vec::new();
        for size in SIZES {
            let provider = test_provider();
            let meter = provider.meter("user-event-test");
            let counter = meter.u64_counter("counter_single_large").build();
            counter.add(1, &[KeyValue::new("payload", "x".repeat(size))]);

            let decoded = test_utils::collect_otlp_metrics_with_pages(1024, || {
                // Export returns an error when a data point cannot be encoded
                // within the limit; that is expected for the larger sizes here,
                // so the result is deliberately not unwrapped.
                let _ = provider.shutdown();
            });

            test_utils::assert_all_events_within_size_limit(&decoded);
            let points = test_utils::number_points(&decoded, "counter_single_large");
            delivered.push((size, points.len()));
        }

        println!("single data point delivery by attribute size: {delivered:?}");

        assert_eq!(
            delivered[0].1, 1,
            "a 4 KiB data point must fit in one event"
        );
        assert_eq!(
            delivered.last().expect("sizes must not be empty").1,
            0,
            "a 16 KiB data point cannot fit and must be dropped rather than written"
        );

        let mut seen_drop = false;
        for (size, count) in &delivered {
            if *count == 0 {
                seen_drop = true;
            } else {
                assert!(
                    !seen_drop,
                    "size {size} was delivered after a smaller size was dropped; \
                     the size accounting is not monotonic"
                );
            }
        }
    }

    /// Every split event must repeat the full scope and resource identity, not
    /// just the scope name.
    ///
    /// A consumer receives these events independently and cannot reconstruct a
    /// missing version or schema URL from a neighbouring event, so dropping
    /// them on continuation events would silently change how the data is
    /// attributed.
    #[ignore]
    #[test]
    fn integration_test_scope_and_resource_identity_repeated_on_every_split_event() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        const SCHEMA_URL: &str = "https://opentelemetry.io/schemas/1.30.0";
        const SERIES: usize = 900;

        let provider = SdkMeterProvider::builder()
            .with_resource(
                Resource::builder_empty()
                    .with_attributes(vec![KeyValue::new("service.name", "metric-demo")])
                    .with_schema_url(Vec::<KeyValue>::new(), SCHEMA_URL)
                    .build(),
            )
            .with_periodic_exporter(MetricsExporter::new())
            .build();

        let meter = provider.meter_with_scope(
            opentelemetry::InstrumentationScope::builder("scoped-meter")
                .with_version("4.5.6")
                .with_schema_url(SCHEMA_URL)
                .build(),
        );
        let counter = meter.u64_counter("counter_scope_identity").build();
        for i in 0..SERIES {
            counter.add(1, &[KeyValue::new("partition", format!("p{i:04}"))]);
        }

        let decoded = test_utils::collect_otlp_metrics_with_pages(1024, || {
            provider.shutdown().expect("shutdown failed");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        assert!(
            decoded.len() > 1,
            "expected the metric to split across several events, got {}",
            decoded.len()
        );
        assert_eq!(
            test_utils::number_points(&decoded, "counter_scope_identity").len(),
            SERIES,
            "data points were lost while splitting"
        );

        for (index, request) in decoded.iter().enumerate() {
            let resource_metrics = &request.resource_metrics;
            assert_eq!(resource_metrics.len(), 1, "event {index}");
            let rm = &resource_metrics[0];
            assert_eq!(
                rm.schema_url, SCHEMA_URL,
                "event {index} lost the resource schema URL"
            );
            assert!(
                rm.resource.is_some(),
                "event {index} lost the resource entirely"
            );

            assert_eq!(rm.scope_metrics.len(), 1, "event {index}");
            let sm = &rm.scope_metrics[0];
            assert_eq!(
                sm.schema_url, SCHEMA_URL,
                "event {index} lost the scope schema URL"
            );
            let scope = sm.scope.as_ref().expect("scope missing");
            assert_eq!(scope.name, "scoped-meter", "event {index}");
            assert_eq!(
                scope.version, "4.5.6",
                "event {index} lost the scope version"
            );
        }
    }

    /// An instrument that reports no observations must not produce an event,
    /// and must not disturb a populated metric exported in the same cycle.
    ///
    /// This asserts the externally visible behaviour only. SDK 0.32 drops a
    /// metric whose aggregation produced no data points before calling the
    /// exporter, so the exporter's empty-batch path is not reached from here;
    /// this test exists to notice if that ever stops being true.
    #[ignore]
    #[test]
    fn integration_test_observable_with_no_observations_emits_no_event() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        let provider = test_provider();
        let meter = provider.meter("user-event-test");

        let _empty = meter
            .u64_observable_counter("counter_never_observed")
            .with_callback(|_observer| {
                // Deliberately reports nothing.
            })
            .build();

        meter
            .u64_counter("counter_populated")
            .build()
            .add(7, &[KeyValue::new("partition", "only")]);

        let decoded = test_utils::collect_otlp_metrics_with_pages(1024, || {
            provider.shutdown().expect("shutdown failed");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        assert!(
            test_utils::find_metrics(&decoded, "counter_never_observed").is_empty(),
            "an instrument with no observations must not be exported"
        );

        let points = test_utils::number_points(&decoded, "counter_populated");
        assert_eq!(
            points,
            vec![(
                vec![("partition".to_string(), "only".to_string())],
                test_utils::Num::I(7)
            )],
            "the populated metric was altered by the empty instrument"
        );
    }

    /// Pins the exact byte at which a single data point stops being emitted.
    ///
    /// The monotonicity test above only brackets the cutoff; it would still
    /// pass if the exporter were wildly over-conservative and silently dropped
    /// points that the kernel would have accepted. This binary-searches the
    /// exact attribute size at which delivery stops, then asserts that the
    /// largest delivered event genuinely fills the budget. That is what proves
    /// the accounting is tight rather than merely safe.
    #[ignore]
    #[test]
    fn integration_test_single_data_point_cutoff_is_exact() {
        use prost::Message;

        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        // Returns the encoded size of the delivered event, or None if the data
        // point was dropped.
        let probe = |size: usize| -> Option<usize> {
            let provider = test_provider();
            let meter = provider.meter("user-event-test");
            let counter = meter.u64_counter("counter_cutoff").build();
            counter.add(1, &[KeyValue::new("payload", "x".repeat(size))]);

            let decoded = test_utils::collect_otlp_metrics_with_pages(1024, || {
                // A data point that cannot be encoded within the limit is
                // reported as an export error, which is expected here.
                let _ = provider.shutdown();
            });

            test_utils::assert_all_events_within_size_limit(&decoded);
            if test_utils::number_points(&decoded, "counter_cutoff").is_empty() {
                None
            } else {
                assert_eq!(decoded.len(), 1, "size {size} produced more than one event");
                Some(decoded[0].encoded_len())
            }
        };

        let mut lo = 1024;
        let mut hi = 16384;
        assert!(probe(lo).is_some(), "a 1 KiB data point must be delivered");
        assert!(probe(hi).is_none(), "a 16 KiB data point must be dropped");

        // Invariant: `lo` is always delivered and `hi` is always dropped.
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if probe(mid).is_some() {
                lo = mid;
            } else {
                hi = mid;
            }
        }

        let largest = probe(lo).expect("the search invariant guarantees lo is delivered");
        println!("cutoff: attribute size {lo} delivered as a {largest} byte event, {hi} dropped");

        assert!(
            largest <= crate::exporter::MAX_EVENT_SIZE,
            "the largest delivered event is {largest} bytes, over the {} byte budget",
            crate::exporter::MAX_EVENT_SIZE
        );

        // Growing the attribute by one byte grows the encoded request by one
        // byte (plus at most a few bytes of varint growth in the enclosing
        // length delimiters), so a tight implementation must land within
        // `SIZE_SLACK` of the budget. A larger gap means usable space is being
        // given away and near-limit data points are dropped unnecessarily.
        let headroom = crate::exporter::MAX_EVENT_SIZE - largest;
        assert!(
            headroom <= crate::exporter::SIZE_SLACK,
            "the largest deliverable data point leaves {headroom} bytes of the {} byte \
             budget unused, which is more than the {} bytes of slack the encoder reserves; \
             the size accounting is over-conservative and is dropping valid data points",
            crate::exporter::MAX_EVENT_SIZE,
            crate::exporter::SIZE_SLACK
        );
    }

    /// Pins the exact byte at which a second data point is pushed into a new
    /// event, and proves nothing is lost at that split.
    ///
    /// This is the counterpart to the test above: that one exercises the
    /// "single point does not fit at all" path, this one exercises the "batch
    /// is full, start another event" path at single-byte granularity.
    #[ignore]
    #[test]
    fn integration_test_batch_split_boundary_is_exact() {
        use prost::Message;

        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        // Emits two equally sized data points and returns how many events they
        // were spread across, asserting no loss either way.
        let probe = |size: usize| -> usize {
            let provider = test_provider();
            let meter = provider.meter("user-event-test");
            let counter = meter.u64_counter("counter_split_cutoff").build();
            counter.add(
                1,
                &[KeyValue::new("payload", format!("a{}", "x".repeat(size)))],
            );
            counter.add(
                1,
                &[KeyValue::new("payload", format!("b{}", "x".repeat(size)))],
            );

            let decoded = test_utils::collect_otlp_metrics_with_pages(1024, || {
                provider.shutdown().expect("shutdown failed");
            });

            test_utils::assert_all_events_within_size_limit(&decoded);
            let points = test_utils::number_points(&decoded, "counter_split_cutoff");
            assert_eq!(
                points.len(),
                2,
                "size {size}: both data points fit individually, so neither may be lost \
                 regardless of how they are split across {} event(s)",
                decoded.len()
            );

            // Counting is not enough: losing one point and duplicating the
            // other also yields two. The two payloads are distinguishable by
            // their first byte, so assert exactly one of each survived intact.
            let mut prefixes: Vec<char> = points
                .iter()
                .map(|(attrs, value)| {
                    assert_eq!(
                        *value,
                        test_utils::Num::I(1),
                        "size {size}: data point value was altered"
                    );
                    attrs
                        .iter()
                        .find(|(k, _)| k == "payload")
                        .map(|(_, v)| v.chars().next().expect("payload is never empty"))
                        .expect("payload attribute missing")
                })
                .collect();
            prefixes.sort_unstable();
            assert_eq!(
                prefixes,
                vec!['a', 'b'],
                "size {size}: expected both distinct data points to survive the split, \
                 got payload prefixes {prefixes:?}"
            );

            decoded.len()
        };

        // At 512 bytes each both points share one event; at 4096 bytes each
        // they cannot.
        let mut lo = 512;
        let mut hi = 4096;
        assert_eq!(
            probe(lo),
            1,
            "two 512 byte data points must share one event"
        );
        assert_eq!(probe(hi), 2, "two 4 KiB data points cannot share one event");

        // Invariant: `lo` fits in one event, `hi` requires two.
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if probe(mid) == 1 {
                lo = mid;
            } else {
                hi = mid;
            }
        }

        println!(
            "split boundary: two {lo} byte points share an event, two {hi} byte points do not"
        );

        // Confirm the boundary is where the encoder says it is: the combined
        // event at `lo` must genuinely be near the budget, not split early.
        let provider = test_provider();
        let meter = provider.meter("user-event-test");
        let counter = meter.u64_counter("counter_split_full").build();
        counter.add(
            1,
            &[KeyValue::new("payload", format!("a{}", "x".repeat(lo)))],
        );
        counter.add(
            1,
            &[KeyValue::new("payload", format!("b{}", "x".repeat(lo)))],
        );
        let decoded = test_utils::collect_otlp_metrics_with_pages(1024, || {
            provider.shutdown().expect("shutdown failed");
        });
        assert_eq!(decoded.len(), 1);
        let filled = decoded[0].encoded_len();
        let headroom = crate::exporter::MAX_EVENT_SIZE - filled;
        assert!(
            headroom <= crate::exporter::SIZE_SLACK,
            "the last two data points to share an event only filled {filled} of the {} byte \
             budget, leaving {headroom} bytes unused; the batch is being split early",
            crate::exporter::MAX_EVENT_SIZE
        );
    }

    // ---------------------------------------------------------------------
    // Value and attribute edge cases.
    // ---------------------------------------------------------------------

    /// Extreme numeric values must round-trip through the protobuf encoding.
    #[ignore]
    #[test]
    fn integration_test_extreme_numeric_values() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        let provider = test_provider();
        let meter = provider.meter("user-event-test");

        let big = meter.u64_counter("counter_u64_max").build();
        big.add(u64::MAX, &[KeyValue::new("k", "max")]);

        let representable = meter.u64_counter("counter_u64_representable").build();
        representable.add(i64::MAX as u64, &[KeyValue::new("k", "max_i64")]);

        let small = meter.i64_up_down_counter("updown_i64_min").build();
        small.add(i64::MIN, &[KeyValue::new("k", "min")]);

        let gauge = meter.f64_gauge("gauge_special_floats").build();
        gauge.record(f64::INFINITY, &[KeyValue::new("k", "inf")]);
        gauge.record(f64::NEG_INFINITY, &[KeyValue::new("k", "neg_inf")]);
        gauge.record(f64::NAN, &[KeyValue::new("k", "nan")]);
        gauge.record(f64::MIN_POSITIVE, &[KeyValue::new("k", "min_positive")]);

        let decoded = test_utils::collect_otlp_metrics(|| {
            provider.shutdown().expect("shutdown failed");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);

        // OTLP represents integral data points as a signed 64-bit value, so a
        // u64 above i64::MAX has no faithful representation. `opentelemetry-proto`
        // clamps those to zero, and this exporter matches it so that the same
        // counter is reported identically over OTLP and over `user_events`. The
        // assertion is written against the value a consumer actually sees: a
        // reinterpreted bit pattern would surface as a negative monotonic
        // counter, which is worse than a zero.
        let big_points = test_utils::number_points(&decoded, "counter_u64_max");
        assert_eq!(big_points.len(), 1);
        assert_eq!(
            big_points[0].1,
            test_utils::Num::I(0),
            "a u64 above i64::MAX must not be written as a negative value"
        );

        let representable = test_utils::number_points(&decoded, "counter_u64_representable");
        assert_eq!(representable.len(), 1);
        assert_eq!(
            representable[0].1,
            test_utils::Num::I(i64::MAX),
            "the largest representable u64 must round-trip exactly"
        );

        let small_points = test_utils::number_points(&decoded, "updown_i64_min");
        assert_eq!(small_points.len(), 1);
        assert_eq!(small_points[0].1, test_utils::Num::I(i64::MIN));

        let floats = test_utils::number_points(&decoded, "gauge_special_floats");
        assert_eq!(floats.len(), 4, "every gauge series must be exported");
        for (attrs, value) in &floats {
            let key = attrs
                .iter()
                .find(|(k, _)| k == "k")
                .map(|(_, v)| v.clone())
                .expect("attribute k missing");
            let test_utils::Num::D(d) = value else {
                panic!("expected a double data point for {key}");
            };
            match key.as_str() {
                "inf" => assert!(d.is_infinite() && d.is_sign_positive()),
                "neg_inf" => assert!(d.is_infinite() && d.is_sign_negative()),
                // NaN is never equal to itself, so this cannot be asserted with
                // a plain comparison.
                "nan" => assert!(d.is_nan(), "NaN must survive as NaN"),
                "min_positive" => assert_eq!(*d, f64::MIN_POSITIVE),
                other => panic!("unexpected series {other}"),
            }
        }
    }

    /// Attribute keys and values are copied verbatim into the payload, so
    /// multi-byte and empty values must not be mangled or truncated.
    #[ignore]
    #[test]
    fn integration_test_unicode_and_empty_attribute_values() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        let cases: Vec<(&str, String)> = vec![
            ("ascii", "plain".to_string()),
            ("empty", String::new()),
            ("cjk", "配置更新".to_string()),
            ("emoji", "🚀🛰️".to_string()),
            ("combining", "e\u{0301}".to_string()),
            ("whitespace", " leading and trailing ".to_string()),
            ("newline", "line1\nline2".to_string()),
            ("quotes", "he said \"hi\"".to_string()),
        ];

        let provider = test_provider();
        let meter = provider.meter("user-event-test");
        let counter = meter.u64_counter("counter_unicode").build();
        for (name, value) in &cases {
            counter.add(
                1,
                &[
                    KeyValue::new("case", *name),
                    KeyValue::new("value", value.clone()),
                ],
            );
        }

        let decoded = test_utils::collect_otlp_metrics(|| {
            provider.shutdown().expect("shutdown failed");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        let points = test_utils::number_points(&decoded, "counter_unicode");
        assert_eq!(points.len(), cases.len(), "every series must be exported");

        for (name, expected) in &cases {
            let found = points
                .iter()
                .find(|(attrs, _)| attrs.iter().any(|(k, v)| k == "case" && v == name))
                .unwrap_or_else(|| panic!("series {name} was not exported"));
            let actual = found
                .0
                .iter()
                .find(|(k, _)| k == "value")
                .map(|(_, v)| v.clone())
                .expect("value attribute missing");
            assert_eq!(&actual, expected, "series {name} was mangled in transit");
        }
    }

    /// A data point carrying many attributes. The attribute list is the bulk of
    /// a data point's size, so this is the shape most likely to interact badly
    /// with the per-point size accounting.
    #[ignore]
    #[test]
    fn integration_test_many_attributes_per_data_point() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        const ATTRS: usize = 32;
        const SERIES: usize = 50;

        let provider = test_provider();
        let meter = provider.meter("user-event-test");
        let counter = meter.u64_counter("counter_wide").build();
        for i in 0..SERIES {
            let mut kvs: Vec<KeyValue> = (0..ATTRS)
                .map(|a| KeyValue::new(format!("dimension_{a:02}"), format!("value_{a:02}")))
                .collect();
            kvs.push(KeyValue::new("partition", format!("p{i:05}")));
            counter.add(1, &kvs);
        }

        let decoded = test_utils::collect_otlp_metrics_with_pages(1024, || {
            provider.shutdown().expect("shutdown failed");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        test_utils::assert_one_metric_per_event(&decoded);

        let points = test_utils::number_points(&decoded, "counter_wide");
        assert_eq!(
            points.len(),
            SERIES,
            "every wide data point must be exported"
        );
        for (attrs, _) in &points {
            assert_eq!(
                attrs.len(),
                ATTRS + 1,
                "a wide data point lost attributes during batching"
            );
        }
    }

    /// Histograms must also report deltas rather than cumulative totals across
    /// collection cycles, including the bucket counts.
    #[ignore]
    #[test]
    fn integration_test_histogram_delta_across_cycles() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        let provider = test_provider();
        let meter = provider.meter("user-event-test");
        let hist = meter.f64_histogram("histogram_delta").build();
        let attrs = [KeyValue::new("k", "a")];

        hist.record(1.0, &attrs);
        hist.record(2.0, &attrs);
        let first = test_utils::collect_otlp_metrics(|| {
            provider.force_flush().expect("first force_flush failed");
        });
        let first_points = test_utils::histogram_points(&first, "histogram_delta");
        assert_eq!(first_points.len(), 1);
        assert_eq!(
            first_points[0].1.count, 2,
            "first cycle recorded two values"
        );
        assert_eq!(first_points[0].1.sum, Some(3.0));

        hist.record(10.0, &attrs);
        let second = test_utils::collect_otlp_metrics(|| {
            provider.force_flush().expect("second force_flush failed");
        });
        let second_points = test_utils::histogram_points(&second, "histogram_delta");
        assert_eq!(second_points.len(), 1);
        assert_eq!(
            second_points[0].1.count, 1,
            "delta temporality means the second cycle reports only the new measurement"
        );
        assert_eq!(second_points[0].1.sum, Some(10.0));
        assert_eq!(
            second_points[0].1.bucket_counts.iter().sum::<u64>(),
            1,
            "bucket counts must be deltas too"
        );

        provider.shutdown().expect("shutdown failed");
    }

    /// All four data point types in a single collection cycle, each large enough
    /// to span multiple events. This is the closest thing to a production export
    /// in the suite.
    #[ignore]
    #[test]
    fn integration_test_mixed_instrument_types_in_one_cycle() {
        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        const SERIES: usize = 400;

        let provider = test_provider();
        let meter = provider.meter("user-event-test");
        let counter = meter.u64_counter("mixed_counter").build();
        let udc = meter.i64_up_down_counter("mixed_updown").build();
        let hist = meter.f64_histogram("mixed_histogram").build();
        meter
            .u64_observable_gauge("mixed_gauge")
            .with_callback(|obs| {
                for i in 0..SERIES {
                    obs.observe(i as u64, &[KeyValue::new("partition", format!("p{i:05}"))]);
                }
            })
            .build();

        for i in 0..SERIES {
            let attrs = [KeyValue::new("partition", format!("p{i:05}"))];
            counter.add(1, &attrs);
            udc.add(-1, &attrs);
            hist.record(i as f64, &attrs);
        }

        let decoded = test_utils::collect_otlp_metrics_with_pages(2048, || {
            provider.shutdown().expect("shutdown failed");
        });

        test_utils::assert_all_events_within_size_limit(&decoded);
        test_utils::assert_envelope_repeated(&decoded, RESOURCE_ATTRS, "user-event-test");
        test_utils::assert_one_metric_per_event(&decoded);

        assert_eq!(
            test_utils::number_points(&decoded, "mixed_counter").len(),
            SERIES
        );
        assert_eq!(
            test_utils::number_points(&decoded, "mixed_updown").len(),
            SERIES
        );
        assert_eq!(
            test_utils::number_points(&decoded, "mixed_gauge").len(),
            SERIES
        );
        assert_eq!(
            test_utils::histogram_points(&decoded, "mixed_histogram").len(),
            SERIES
        );

        // Each metric must appear in at least one event, and no event may mix
        // data points from two different metrics.
        for name in [
            "mixed_counter",
            "mixed_updown",
            "mixed_gauge",
            "mixed_histogram",
        ] {
            assert!(
                !test_utils::find_metrics(&decoded, name).is_empty(),
                "{name} was never exported"
            );
        }
    }

    /// Pins the mapping from the SDK's `Temporality` to the OTLP wire value.
    ///
    /// The two enums are numbered differently (`Cumulative` is 0 in the SDK but
    /// 2 in OTLP, where 0 means UNSPECIFIED), so a direct cast is wrong for
    /// everything except delta. This asserts the actual bytes a consumer reads
    /// for both a delta instrument and a cumulative one.
    #[ignore]
    #[test]
    fn integration_test_temporality_uses_otlp_wire_values() {
        use opentelemetry_proto::tonic::metrics::v1::metric::Data;
        use opentelemetry_proto::tonic::metrics::v1::AggregationTemporality;

        test_utils::check_user_events_available().expect("Kernel does not support user_events.");

        let provider = test_provider();
        let meter = provider.meter("user-event-test");
        meter
            .u64_counter("temporality_counter")
            .build()
            .add(1, &[KeyValue::new("k", "a")]);
        meter
            .i64_up_down_counter("temporality_updown")
            .build()
            .add(1, &[KeyValue::new("k", "a")]);
        meter
            .f64_histogram("temporality_histogram")
            .build()
            .record(1.0, &[KeyValue::new("k", "a")]);

        let decoded = test_utils::collect_otlp_metrics(|| {
            provider.shutdown().expect("shutdown failed");
        });

        let temporality_of = |name: &str| -> i32 {
            let metrics = test_utils::find_metrics(&decoded, name);
            assert!(!metrics.is_empty(), "{name} was never exported");
            match metrics[0].data.as_ref().expect("metric data missing") {
                Data::Sum(s) => s.aggregation_temporality,
                Data::Histogram(h) => h.aggregation_temporality,
                other => panic!("{name} has unexpected data {other:?}"),
            }
        };

        // A monotonic counter is aggregated as delta, which is 1 in both enums.
        assert_eq!(
            temporality_of("temporality_counter"),
            AggregationTemporality::Delta as i32
        );
        assert_eq!(
            temporality_of("temporality_histogram"),
            AggregationTemporality::Delta as i32
        );

        // An up/down counter is aggregated cumulatively, which is 2 on the wire.
        // A direct enum cast would write 0 here, which OTLP defines as
        // UNSPECIFIED.
        let updown = temporality_of("temporality_updown");
        assert_ne!(
            updown,
            AggregationTemporality::Unspecified as i32,
            "cumulative temporality must not be written as UNSPECIFIED"
        );
        assert_eq!(updown, AggregationTemporality::Cumulative as i32);
    }
}
