//! ETW metrics end-to-end integration test.
//!
//! Exercises the public `MetricsExporter` pipeline, starts a real ETW
//! capture session via `one_collect`, reads the raw event payload
//! (the exporter emits manifested events with a `raw_data` payload, so the
//! user buffer is the raw OTLP protobuf), decodes each event as
//! `ExportMetricsServiceRequest`, and asserts the OTLP shape for every
//! supported instrument type (Counter, UpDownCounter, Gauge, Histogram).
//!
//! Run locally (requires admin privileges to start an ETW session):
//!
//! ```text
//! cargo test --manifest-path opentelemetry-etw-metrics/Cargo.toml --all-features \
//!     -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Single-test design: the ETW provider in this crate is registered through
//! a static `std::sync::Once`, so only one register/unregister cycle is
//! supported per process. All instrument coverage lives in one test.

#![cfg(target_os = "windows")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use one_collect::etw::{EtwSession, LEVEL_VERBOSE};
use one_collect::event::os::windows::WindowsEventExtension;
use one_collect::event::Event;
use one_collect::Guid;

use opentelemetry::metrics::MeterProvider as _;
use opentelemetry::KeyValue;
use opentelemetry_etw_metrics::MetricsExporter;
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValueValue;
use opentelemetry_proto::tonic::common::v1::KeyValue as ProtoKeyValue;
use opentelemetry_proto::tonic::metrics::v1::{metric::Data as MetricData, Metric as ProtoMetric};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::Resource;
use prost::Message;

// Must match the provider GUID statically registered in src/etw/mod.rs.
// GUID EDC24920-E004-40F6-A8E1-0E6E48F39D84.
const PROVIDER_GUID: u128 = 0xEDC2_4920_E004_40F6_A8E1_0E6E_48F3_9D84;

/// Handle to a running ETW capture session backed by one_collect.
///
/// `one_collect::etw::EtwSession::parse_until` blocks the calling thread
/// and runs event callbacks on it, so the session is driven on a dedicated
/// worker thread. Dropping (or explicitly `stop`-ping) the handle signals
/// that thread to stop and joins it.
struct EtwTrace {
    stop: Arc<AtomicBool>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
}

impl EtwTrace {
    fn stop(&self) -> thread::Result<()> {
        self.stop.store(true, Ordering::Relaxed);
        match self.worker.lock().unwrap().take() {
            Some(worker) => worker.join(),
            None => Ok(()),
        }
    }
}

impl Drop for EtwTrace {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn start_etw_trace() -> (EtwTrace, mpsc::Receiver<ExportMetricsServiceRequest>) {
    let (tx, rx) = mpsc::sync_channel::<ExportMetricsServiceRequest>(64);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_worker = stop.clone();

    let worker = thread::spawn(move || {
        let mut session = EtwSession::new();

        // Wide event: capture every event from the provider (any ID). The
        // exporter emits a raw OTLP protobuf payload as the user buffer, so
        // `data.event_data()` is the bytes we decode directly.
        let mut event = Event::for_etw(
            0,
            "Event".to_string(),
            Guid::from_u128(PROVIDER_GUID),
            LEVEL_VERBOSE,
            u64::MAX,
        );
        event.set_id_wild_card_flag();

        event.add_callback(move |data| {
            if let Ok(req) = ExportMetricsServiceRequest::decode(data.event_data()) {
                let _ = tx.try_send(req);
            }
            Ok(())
        });

        session.add_event(event, None);

        let _ = session
            .parse_until("etw-metrics-int-test", move || stop_worker.load(Ordering::Relaxed));
    });

    (
        EtwTrace {
            stop,
            worker: Mutex::new(Some(worker)),
        },
        rx,
    )
}

/// Wait up to 5s for the first event, then drain anything that arrives within
/// a 200 ms tail window. Bounded by an overall 5 s deadline.
fn drain_events(
    rx: &mpsc::Receiver<ExportMetricsServiceRequest>,
) -> Vec<ExportMetricsServiceRequest> {
    let mut out = Vec::new();
    let overall_deadline = Instant::now() + Duration::from_secs(5);
    if let Ok(first) = rx.recv_timeout(Duration::from_secs(5)) {
        out.push(first);
    }
    loop {
        let remaining = overall_deadline.saturating_duration_since(Instant::now());
        let wait = remaining.min(Duration::from_millis(200));
        if wait.is_zero() {
            break;
        }
        match rx.recv_timeout(wait) {
            Ok(r) => out.push(r),
            Err(_) => break,
        }
    }
    out
}

fn attr_string(attrs: &[ProtoKeyValue], key: &str) -> String {
    let a = attrs
        .iter()
        .find(|a| a.key == key)
        .unwrap_or_else(|| panic!("attribute '{key}' not found"));
    match a.value.as_ref().and_then(|v| v.value.as_ref()) {
        Some(AnyValueValue::StringValue(s)) => s.clone(),
        _ => panic!("attribute '{key}' is not a string"),
    }
}

#[ignore = "Requires admin privileges to start ETW trace session"]
#[test]
fn integration_test_all_instruments() {
    let (trace, rx) = start_etw_trace();

    // Allow ETW consumer thread to attach before any events are emitted.
    std::thread::sleep(Duration::from_millis(500));

    let exporter = MetricsExporter::new();
    let reader = PeriodicReader::builder(exporter).build();
    let provider = SdkMeterProvider::builder()
        .with_resource(
            Resource::builder_empty()
                .with_attributes(vec![KeyValue::new("service.name", "etw-metrics-int-test")])
                .build(),
        )
        .with_reader(reader)
        .build();
    let meter = provider.meter("etw-metric-test");

    // Counter (u64) — two attribute sets => two events.
    let counter = meter
        .u64_counter("test_counter_u64")
        .with_description("counter test desc")
        .with_unit("counter_unit")
        .build();
    counter.add(1, &[KeyValue::new("k", "a")]);
    counter.add(2, &[KeyValue::new("k", "b")]);

    // UpDownCounter (i64) — aggregated: k=a => +5, k=b => -3.
    let udc = meter
        .i64_up_down_counter("test_updown_i64")
        .with_description("updown test desc")
        .with_unit("updown_unit")
        .build();
    udc.add(10, &[KeyValue::new("k", "a")]);
    udc.add(-5, &[KeyValue::new("k", "a")]);
    udc.add(-3, &[KeyValue::new("k", "b")]);

    // Gauge (u64) — single attribute set => one event with value 42.
    let gauge = meter
        .u64_gauge("test_gauge_u64")
        .with_description("gauge test desc")
        .with_unit("gauge_unit")
        .build();
    gauge.record(42, &[KeyValue::new("k", "a")]);

    // Histogram (f64) — three observations on one attribute set:
    // count=3, sum=16.0, min=1.0, max=10.0.
    let hist = meter
        .f64_histogram("test_hist_f64")
        .with_description("hist test desc")
        .with_unit("hist_unit")
        .build();
    let h_attrs = [KeyValue::new("k", "a")];
    hist.record(1.0, &h_attrs);
    hist.record(5.0, &h_attrs);
    hist.record(10.0, &h_attrs);

    // Shutdown forces a final export and unregisters the ETW provider.
    provider.shutdown().expect("provider.shutdown failed");

    let events = drain_events(&rx);
    // Always stop the trace, even if assertions panic below.
    let _ = trace.stop();

    assert!(
        !events.is_empty(),
        "expected at least one ETW event after shutdown"
    );

    // Each exported event carries exactly one resource_metrics/scope_metrics/metric.
    // Bucket the decoded metrics by name.
    let mut by_metric: HashMap<&str, Vec<&ProtoMetric>> = HashMap::new();
    for req in &events {
        for rm in &req.resource_metrics {
            let svc = rm.resource.as_ref().and_then(|r| {
                r.attributes
                    .iter()
                    .find(|a| a.key == "service.name")
                    .and_then(|a| a.value.as_ref())
                    .and_then(|v| v.value.as_ref())
                    .and_then(|v| match v {
                        AnyValueValue::StringValue(s) => Some(s.as_str()),
                        _ => None,
                    })
            });
            assert_eq!(svc, Some("etw-metrics-int-test"));
            for sm in &rm.scope_metrics {
                assert_eq!(
                    sm.scope.as_ref().map(|s| s.name.as_str()),
                    Some("etw-metric-test")
                );
                for m in &sm.metrics {
                    by_metric.entry(m.name.as_str()).or_default().push(m);
                }
            }
        }
    }

    // Counter assertions.
    {
        let metrics = by_metric
            .remove("test_counter_u64")
            .expect("counter metric missing");
        assert_eq!(
            metrics.len(),
            2,
            "counter expected 2 events (one per attribute set)"
        );
        let mut points: Vec<(i64, String)> = metrics
            .iter()
            .map(|m| {
                let sum = match m.data.as_ref().expect("counter has data") {
                    MetricData::Sum(s) => s,
                    _ => panic!("counter must be encoded as Sum"),
                };
                assert!(sum.is_monotonic, "counter Sum must be monotonic");
                assert_eq!(sum.data_points.len(), 1);
                let dp = &sum.data_points[0];
                let v = match dp.value.as_ref().expect("dp value missing") {
                    opentelemetry_proto::tonic::metrics::v1::number_data_point::Value::AsInt(v) => {
                        *v
                    }
                    other => panic!("counter value must be int, got {other:?}"),
                };
                assert_eq!(m.description, "counter test desc");
                assert_eq!(m.unit, "counter_unit");
                (v, attr_string(&dp.attributes, "k"))
            })
            .collect();
        points.sort();
        assert_eq!(points, vec![(1, "a".into()), (2, "b".into())]);
    }

    // UpDownCounter assertions.
    {
        let metrics = by_metric
            .remove("test_updown_i64")
            .expect("updown metric missing");
        assert_eq!(metrics.len(), 2);
        let mut points: Vec<(i64, String)> = metrics
            .iter()
            .map(|m| {
                let sum = match m.data.as_ref().expect("updown has data") {
                    MetricData::Sum(s) => s,
                    _ => panic!("updowncounter must be encoded as Sum"),
                };
                assert!(!sum.is_monotonic, "updowncounter Sum must be non-monotonic");
                assert_eq!(sum.data_points.len(), 1);
                let dp = &sum.data_points[0];
                let v = match dp.value.as_ref().expect("dp value missing") {
                    opentelemetry_proto::tonic::metrics::v1::number_data_point::Value::AsInt(v) => {
                        *v
                    }
                    other => panic!("updown value must be int, got {other:?}"),
                };
                (v, attr_string(&dp.attributes, "k"))
            })
            .collect();
        points.sort();
        assert_eq!(points, vec![(-3, "b".into()), (5, "a".into())]);
    }

    // Gauge assertions.
    {
        let metrics = by_metric
            .remove("test_gauge_u64")
            .expect("gauge metric missing");
        assert_eq!(metrics.len(), 1);
        let m = metrics[0];
        let gauge = match m.data.as_ref().expect("gauge has data") {
            MetricData::Gauge(g) => g,
            _ => panic!("gauge must be encoded as Gauge"),
        };
        assert_eq!(gauge.data_points.len(), 1);
        let dp = &gauge.data_points[0];
        let v = match dp.value.as_ref().expect("dp value missing") {
            opentelemetry_proto::tonic::metrics::v1::number_data_point::Value::AsInt(v) => *v,
            other => panic!("gauge value must be int, got {other:?}"),
        };
        assert_eq!(v, 42);
        assert_eq!(attr_string(&dp.attributes, "k"), "a");
        assert_eq!(m.description, "gauge test desc");
        assert_eq!(m.unit, "gauge_unit");
    }

    // Histogram assertions.
    {
        let metrics = by_metric
            .remove("test_hist_f64")
            .expect("histogram metric missing");
        assert_eq!(metrics.len(), 1);
        let m = metrics[0];
        let hist = match m.data.as_ref().expect("hist has data") {
            MetricData::Histogram(h) => h,
            _ => panic!("histogram must be encoded as Histogram"),
        };
        assert_eq!(hist.data_points.len(), 1);
        let dp = &hist.data_points[0];
        assert_eq!(dp.count, 3);
        assert_eq!(dp.sum, Some(16.0));
        assert_eq!(dp.min, Some(1.0));
        assert_eq!(dp.max, Some(10.0));
        assert_eq!(dp.bucket_counts.len(), dp.explicit_bounds.len() + 1);
        assert_eq!(dp.bucket_counts.iter().sum::<u64>(), dp.count);
        assert_eq!(attr_string(&dp.attributes, "k"), "a");
        assert_eq!(m.description, "hist test desc");
        assert_eq!(m.unit, "hist_unit");
    }

    assert!(
        by_metric.is_empty(),
        "unexpected extra metrics: {:?}",
        by_metric.keys().collect::<Vec<_>>()
    );
}
