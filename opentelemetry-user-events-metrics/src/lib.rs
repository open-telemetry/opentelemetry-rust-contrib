mod exporter;
mod tracepoint;

pub use exporter::MetricsExporter;

/// Empirical measurement of the maximum `user_events` event size that is
/// actually delivered to a consumer, for both the perf and ftrace paths.
///
/// This module is an experiment, not a regression test. It exists to answer a
/// specific question: the `eventheader` crate documents that "the system will
/// ignore any event that is larger than 64KB", but `user_event_perf()` in the
/// kernel stages records through `perf_trace_buf_alloc()`, which refuses
/// anything above `PERF_MAX_TRACE_SIZE` (8192). Those two claims imply very
/// different budgets for a batching exporter, so this measures the boundary
/// directly instead of arguing from source.
///
/// Run with:
///   sudo -E cargo test --lib size_experiment -- --ignored --nocapture --test-threads=1
#[cfg(all(test, target_os = "linux"))]
mod size_experiment {
    use eventheader::_internal as ehi;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Payload sizes to probe, in bytes. Chosen to bracket both candidate
    /// bounds: the ftrace sub-buffer (~4 KiB), `PERF_MAX_TRACE_SIZE` (8192),
    /// and the 64 KiB perf record / ABI ceiling.
    const PROBE_SIZES: &[usize] = &[
        512, 1024, 2048, 3072, 4000, 4048, 4072, 4096, 5120, 6144, 8000, 8144, 8168, 8176, 8192,
        8208, 10240, 12288, 16384, 24576, 32768, 49152, 60000, 65000, 65360, 65500,
    ];

    /// Number of times each size is written, so a single transient drop is not
    /// mistaken for a hard limit.
    const REPEATS: usize = 3;

    fn tracefs_root() -> PathBuf {
        for candidate in ["/sys/kernel/tracing", "/sys/kernel/debug/tracing"] {
            if Path::new(candidate).join("events").is_dir() {
                return PathBuf::from(candidate);
            }
        }
        panic!("tracefs is not mounted; run as root on a kernel with tracefs available");
    }

    /// Builds a payload of `size` bytes whose first 8 bytes encode `size`, so a
    /// received event can be attributed back to the probe that produced it.
    fn payload(size: usize) -> Vec<u8> {
        let mut buffer = vec![0xABu8; size];
        let tag = (size as u64).to_le_bytes();
        let n = tag.len().min(size);
        buffer[..n].copy_from_slice(&tag[..n]);
        buffer
    }

    fn print_environment(root: &Path) {
        println!("--- environment ---");
        println!("page_size: {}", unsafe {
            libc_sysconf_page_size().unwrap_or(0)
        });
        for file in ["buffer_subbuf_size_kb", "buffer_size_kb"] {
            let path = root.join(file);
            match fs::read_to_string(&path) {
                Ok(v) => println!("{file}: {}", v.trim()),
                Err(e) => println!("{file}: <unavailable: {e}>"),
            }
        }
        match fs::read_to_string("/proc/sys/kernel/perf_event_paranoid") {
            Ok(v) => println!("perf_event_paranoid: {}", v.trim()),
            Err(e) => println!("perf_event_paranoid: <unavailable: {e}>"),
        }
        if let Ok(v) = fs::read_to_string("/proc/version") {
            println!("kernel: {}", v.trim());
        }
    }

    /// `sysconf(_SC_PAGESIZE)` without pulling in a new dependency.
    unsafe fn libc_sysconf_page_size() -> Option<i64> {
        unsafe extern "C" {
            fn sysconf(name: i32) -> i64;
        }
        // _SC_PAGESIZE is 30 on Linux.
        let value = unsafe { sysconf(30) };
        if value > 0 {
            Some(value)
        } else {
            None
        }
    }

    /// Registers the `otlp_metrics` tracepoint and returns it. Registration is
    /// what makes the event visible in tracefs, which both probes depend on.
    fn register_tracepoint() -> std::pin::Pin<Box<ehi::TracepointState>> {
        let trace_point = Box::pin(ehi::TracepointState::new(0));
        // Safety: the tracepoint lives for the rest of the test process and is
        // unregistered when dropped.
        unsafe {
            let result = crate::tracepoint::register(trace_point.as_ref());
            assert_eq!(result, 0, "failed to register otlp_metrics tracepoint");
        }
        trace_point
    }

    /// Sums the `entries` counter across every per-CPU ring buffer. This counts
    /// records that the kernel actually committed, independent of whether the
    /// `trace` text formatter can render them: an event larger than
    /// `TRACE_SEQ_SIZE` (8192) renders as `[LINE TOO BIG]`, so counting lines in
    /// `trace` measures the formatter rather than delivery.
    fn ftrace_entries(root: &Path) -> usize {
        let mut total = 0;
        let per_cpu = root.join("per_cpu");
        let dir = fs::read_dir(&per_cpu).expect("failed to list per_cpu directory");
        for entry in dir.flatten() {
            let stats = entry.path().join("stats");
            let Ok(text) = fs::read_to_string(&stats) else {
                continue;
            };
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("entries:") {
                    total += rest.trim().parse::<usize>().unwrap_or(0);
                }
            }
        }
        total
    }

    fn report(label: &str, results: &[(usize, usize, usize)]) {
        println!("--- {label} ---");
        println!(
            "{:>8}  {:>9}  {:>8}  verdict",
            "size", "delivered", "written"
        );
        let mut largest_ok = None;
        let mut smallest_dropped = None;
        for (size, delivered, written) in results {
            let verdict = if *delivered == *written {
                // PROBE_SIZES is ascending, so the last match is the largest.
                largest_ok = Some(*size);
                "ok"
            } else if *delivered == 0 {
                if smallest_dropped.is_none() {
                    smallest_dropped = Some(*size);
                }
                "DROPPED"
            } else {
                "PARTIAL"
            };
            println!("{size:>8}  {delivered:>9}  {written:>8}  {verdict}");
        }
        println!(
            "{label}: largest fully delivered = {:?}, smallest fully dropped = {:?}",
            largest_ok, smallest_dropped
        );
    }

    /// Probes the perf delivery path using a one_collect perf ring buffer
    /// session, which is how this crate's integration tests (and, as far as we
    /// know, production consumers built on one_collect) read the tracepoint.
    #[ignore]
    #[test]
    fn size_experiment_perf() {
        use one_collect::perf_event::{RingBufBuilder, RingBufSessionBuilder};
        use one_collect::tracefs::TraceFS;
        use one_collect::Writable;

        let root = tracefs_root();
        print_environment(&root);

        let trace_point = register_tracepoint();

        let tracefs = TraceFS::open().expect("need root to open tracefs");
        let mut event = tracefs
            .find_event("user_events", "otlp_metrics")
            .expect("otlp_metrics tracepoint not found after registration");
        let buffer_ref = event.format().get_field_ref_unchecked("buffer");

        let received = Writable::<Vec<usize>>::new(Vec::new());
        let sink = received.clone();
        event.add_callback(move |data| {
            let buffer = data.format().get_data(buffer_ref, data.event_data());
            // First 8 bytes carry the size the producer intended to write.
            if buffer.len() >= 8 {
                let mut tag = [0u8; 8];
                tag.copy_from_slice(&buffer[..8]);
                sink.write(|out| out.push(u64::from_le_bytes(tag) as usize));
            }
            Ok(())
        });

        // 8 MiB per CPU, far larger than the ~1 MiB this test writes, so ring
        // buffer capacity cannot be confused for a per-event limit.
        let mut session = RingBufSessionBuilder::new()
            .with_page_count(2048)
            .with_tracepoint_events(RingBufBuilder::for_tracepoint())
            .with_target_pid(std::process::id() as i32)
            .build()
            .expect("need root to create a perf session");
        session.add_event(event).expect("failed to add event");
        session.enable().expect("failed to enable perf session");

        assert!(
            trace_point.enabled(),
            "tracepoint should be enabled once the perf session is attached"
        );

        let mut write_codes = Vec::new();
        for &size in PROBE_SIZES {
            let buffer = payload(size);
            for _ in 0..REPEATS {
                let code = crate::tracepoint::write(&trace_point, &buffer);
                write_codes.push((size, code));
            }
        }

        session.disable().expect("failed to disable perf session");
        session
            .parse_all()
            .expect("failed to drain perf ring buffer");

        let mut delivered = Vec::new();
        received.read(|v| delivered = v.clone());

        println!("--- userspace write() return codes (0 == success) ---");
        for (size, code) in &write_codes {
            if *code != 0 {
                println!("size {size}: write returned {code}");
            }
        }
        println!("(only non-zero codes shown)");

        let results: Vec<(usize, usize, usize)> = PROBE_SIZES
            .iter()
            .map(|&size| {
                let count = delivered.iter().filter(|&&s| s == size).count();
                (size, count, REPEATS)
            })
            .collect();
        report("perf", &results);
    }

    /// Probes the ftrace delivery path by enabling the event through tracefs
    /// and reading back the textual trace buffer. This is the path a consumer
    /// that does not use perf would be on, and it is bounded by the ring buffer
    /// sub-buffer size rather than by `PERF_MAX_TRACE_SIZE`.
    #[ignore]
    #[test]
    fn size_experiment_ftrace() {
        let root = tracefs_root();
        print_environment(&root);

        let trace_point = register_tracepoint();

        let enable_path = root.join("events/user_events/otlp_metrics/enable");
        fs::write(&enable_path, "1").unwrap_or_else(|e| {
            panic!("failed to enable {}: {e}", enable_path.display());
        });
        fs::write(root.join("tracing_on"), "1").expect("failed to set tracing_on");

        assert!(
            trace_point.enabled(),
            "tracepoint should be enabled once the ftrace event is enabled"
        );

        let trace_path = root.join("trace");
        let mut results = Vec::new();
        for &size in PROBE_SIZES {
            let buffer = payload(size);
            // Truncate the trace buffer so each probe is measured in isolation.
            fs::write(&trace_path, "").expect("failed to clear trace buffer");

            let mut written = 0;
            for _ in 0..REPEATS {
                let code = crate::tracepoint::write(&trace_point, &buffer);
                if code == 0 {
                    written += 1;
                } else {
                    println!("size {size}: write returned {code}");
                }
            }

            let delivered = ftrace_entries(&root);
            results.push((size, delivered, written));
        }

        let _ = fs::write(&enable_path, "0");
        report("ftrace", &results);
    }
}

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

            event.add_callback(move |data| {
                let buffer = data.format().get_data(buffer_ref, data.event_data());
                match ExportMetricsServiceRequest::decode(buffer) {
                    Ok(request) => sink.write(|out| out.push(request)),
                    Err(e) => eprintln!("Failed to decode OTLP metrics from buffer: {e}"),
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
        /// This is the invariant the whole batching scheme rests on: a perf ring
        /// buffer record length is a `__u16`, so an event that overshoots would be
        /// silently unreadable rather than merely large. Because these payloads
        /// came back out of the kernel, this also proves the bound end to end.
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
    /// preservation. It also proves that a maximally packed event survives the
    /// `__u16` perf record length limit, which is the reason `MAX_EVENT_SIZE`
    /// exists at all.
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
        // ~60 byte data points and a 65360 byte budget this should be a ~1000x
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

        for i in 0..3000 {
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

        assert!(
            partitions.iter().any(|p| p == "small-a"),
            "small data point before the oversized one was lost: {partitions:?}"
        );
        assert!(
            partitions.iter().any(|p| p == "small-b"),
            "small data point after the oversized one was lost: {partitions:?}"
        );
        assert!(
            !partitions.iter().any(|p| p.len() > 1000),
            "oversized data point should have been dropped"
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
}
