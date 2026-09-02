use opentelemetry::{otel_debug, otel_info};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::metrics::v1::metric::Data as ProtoData;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::metrics::data::AggregatedMetrics;
use opentelemetry_sdk::metrics::exporter::PushMetricExporter;
use opentelemetry_sdk::metrics::{
    data::{MetricData, ResourceMetrics},
    Temporality,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::tracepoint;
use eventheader::_internal as ehi;
use prost::Message;
use std::fmt::{Debug, Formatter};
use std::pin::Pin;

const MAX_EVENT_SIZE: usize = 65360;

/// Safety margin, in bytes, reserved when deciding whether another data point
/// fits into the current batch.
///
/// Adding data points grows the protobuf length-delimiter of every enclosing
/// message: `Sum`/`Gauge`/`Histogram` -> `Metric` -> `ScopeMetrics` ->
/// `ResourceMetrics`. Each of those four varints grows by at most 2 bytes as
/// the payload approaches `MAX_EVENT_SIZE`, so 8 bytes is the true bound; 32
/// gives ample headroom while costing nothing measurable in packing density.
const SIZE_SLACK: usize = 32;

/// Abstracts the destination of an encoded OTLP payload.
///
/// Production uses [`TracepointWriter`]. Tests substitute an in-memory writer so
/// the encoding and batching logic can be exercised on any platform, not just
/// Linux hosts with `user_events` available.
trait EventWriter: Send + Sync {
    fn enabled(&self) -> bool;
    fn write(&self, buffer: &[u8]) -> i32;
}

struct TracepointWriter {
    trace_point: Pin<Box<ehi::TracepointState>>,
}

impl TracepointWriter {
    fn new() -> Self {
        let trace_point = Box::pin(ehi::TracepointState::new(0));
        // This is unsafe because if the code is used in a shared object,
        // the event MUST be unregistered before the shared object unloads.
        unsafe {
            let _result = tracepoint::register(trace_point.as_ref());
        }
        TracepointWriter { trace_point }
    }
}

impl EventWriter for TracepointWriter {
    fn enabled(&self) -> bool {
        self.trace_point.enabled()
    }

    fn write(&self, buffer: &[u8]) -> i32 {
        tracepoint::write(&self.trace_point, buffer)
    }
}

/// Number of bytes a single data point contributes to its enclosing
/// `repeated` field: a 1-byte tag (all four data-point fields are field 1),
/// the length varint, and the encoded body.
fn data_point_cost<P: Message>(point: &P) -> usize {
    let len = point.encoded_len();
    1 + varint_len(len) + len
}

fn varint_len(mut value: usize) -> usize {
    let mut len = 1;
    while value >= 0x80 {
        value >>= 7;
        len += 1;
    }
    len
}

trait Numeric: Copy {
    // lossy at large values for u64 and i64 but otlp histograms only handle float values
    fn into_f64(self) -> f64;
    fn into_number_data_point_value(
        self,
    ) -> opentelemetry_proto::tonic::metrics::v1::number_data_point::Value;
}

impl Numeric for u64 {
    fn into_f64(self) -> f64 {
        self as f64
    }

    fn into_number_data_point_value(
        self,
    ) -> opentelemetry_proto::tonic::metrics::v1::number_data_point::Value {
        opentelemetry_proto::tonic::metrics::v1::number_data_point::Value::AsInt(self as i64)
    }
}

impl Numeric for i64 {
    fn into_f64(self) -> f64 {
        self as f64
    }

    fn into_number_data_point_value(
        self,
    ) -> opentelemetry_proto::tonic::metrics::v1::number_data_point::Value {
        opentelemetry_proto::tonic::metrics::v1::number_data_point::Value::AsInt(self)
    }
}

impl Numeric for f64 {
    fn into_f64(self) -> f64 {
        self
    }

    fn into_number_data_point_value(
        self,
    ) -> opentelemetry_proto::tonic::metrics::v1::number_data_point::Value {
        opentelemetry_proto::tonic::metrics::v1::number_data_point::Value::AsDouble(self)
    }
}

pub struct MetricsExporter {
    writer: Box<dyn EventWriter>,
    max_event_size: usize,
}

impl MetricsExporter {
    pub fn new() -> MetricsExporter {
        MetricsExporter {
            writer: Box::new(TracepointWriter::new()),
            max_event_size: MAX_EVENT_SIZE,
        }
    }
}

impl Default for MetricsExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for MetricsExporter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("user_events metrics exporter")
    }
}

fn to_nanos(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_nanos() as u64
}

impl MetricsExporter {
    fn process_numeric_metrics<T: Numeric>(
        &self,
        request: &mut ExportMetricsServiceRequest,
        byte_array: &mut Vec<u8>,
        metric: &opentelemetry_sdk::metrics::data::Metric,
        data: &MetricData<T>,
    ) -> usize {
        match data {
            MetricData::Gauge(gauge) => self.process_gauge(request, byte_array, metric, gauge),
            MetricData::Sum(sum) => self.process_sum(request, byte_array, metric, sum),
            MetricData::Histogram(hist) => {
                self.process_histogram(request, byte_array, metric, hist)
            }
            MetricData::ExponentialHistogram(hist) => {
                self.process_exponential_histogram(request, byte_array, metric, hist)
            }
        }
    }

    /// Installs a `Metric` shell (name/description/unit, no data) as the single
    /// metric of the single scope in `request`.
    fn set_metric_shell(
        request: &mut ExportMetricsServiceRequest,
        metric: &opentelemetry_sdk::metrics::data::Metric,
    ) {
        request.resource_metrics[0].scope_metrics[0].metrics =
            vec![opentelemetry_proto::tonic::metrics::v1::Metric {
                name: metric.name().to_string(),
                description: metric.description().to_string(),
                unit: metric.unit().to_string(),
                metadata: vec![],
                data: None,
            }];
    }

    /// Packs `points` into as few tracepoint events as possible.
    ///
    /// Data points are accumulated until adding one more would push the encoded
    /// `ExportMetricsServiceRequest` past `max_event_size`, at which point the
    /// batch is flushed and a new one is started with the remaining points. The
    /// resource, scope and metric metadata (the "envelope") is therefore written
    /// once per event rather than once per data point.
    ///
    /// Returns the number of data points that could not be exported.
    fn emit_batched<P, F>(
        &self,
        request: &mut ExportMetricsServiceRequest,
        byte_array: &mut Vec<u8>,
        metric: &opentelemetry_sdk::metrics::data::Metric,
        points: impl Iterator<Item = P>,
        make_data: F,
    ) -> usize
    where
        P: Message,
        F: Fn(Vec<P>) -> ProtoData,
    {
        // Encode the envelope once (metric present, zero data points) to learn
        // the fixed per-event cost.
        request.resource_metrics[0].scope_metrics[0].metrics[0].data = Some(make_data(Vec::new()));
        let envelope_len = request.encoded_len();

        let mut batch: Vec<P> = Vec::new();
        let mut batch_len = 0usize;
        let mut failed_count = 0usize;

        for point in points {
            let cost = data_point_cost(&point);

            if !batch.is_empty()
                && envelope_len + batch_len + cost + SIZE_SLACK > self.max_event_size
            {
                failed_count +=
                    self.flush_batch(request, byte_array, metric, &mut batch, &make_data);
                batch_len = 0;
            }

            batch_len += cost;
            batch.push(point);
        }

        if !batch.is_empty() {
            failed_count += self.flush_batch(request, byte_array, metric, &mut batch, &make_data);
        }

        failed_count
    }

    /// Encodes and writes the accumulated batch, leaving `batch` empty.
    fn flush_batch<P, F>(
        &self,
        request: &mut ExportMetricsServiceRequest,
        byte_array: &mut Vec<u8>,
        metric: &opentelemetry_sdk::metrics::data::Metric,
        batch: &mut Vec<P>,
        make_data: &F,
    ) -> usize
    where
        P: Message,
        F: Fn(Vec<P>) -> ProtoData,
    {
        let point_count = batch.len();
        request.resource_metrics[0].scope_metrics[0].metrics[0].data =
            Some(make_data(std::mem::take(batch)));

        byte_array.clear();
        if self
            .encode_and_emit_metric(request, byte_array, metric)
            .is_err()
        {
            point_count
        } else {
            0
        }
    }

    fn process_gauge<T: Numeric>(
        &self,
        request: &mut ExportMetricsServiceRequest,
        byte_array: &mut Vec<u8>,
        metric: &opentelemetry_sdk::metrics::data::Metric,
        gauge: &opentelemetry_sdk::metrics::data::Gauge<T>,
    ) -> usize {
        let start_time = gauge.start_time().map(to_nanos).unwrap_or_default();
        let time = to_nanos(gauge.time());
        let default_flags =
            opentelemetry_proto::tonic::metrics::v1::DataPointFlags::default() as u32;

        Self::set_metric_shell(request, metric);

        let points = gauge.data_points().map(|dp| {
            opentelemetry_proto::tonic::metrics::v1::NumberDataPoint {
                attributes: dp.attributes().map(Into::into).collect(),
                start_time_unix_nano: start_time,
                time_unix_nano: time,
                exemplars: Vec::new(), // No support for exemplars
                flags: default_flags,
                value: Some(dp.value().into_number_data_point_value()),
            }
        });

        self.emit_batched(request, byte_array, metric, points, |data_points| {
            ProtoData::Gauge(opentelemetry_proto::tonic::metrics::v1::Gauge { data_points })
        })
    }

    fn process_sum<T: Numeric>(
        &self,
        request: &mut ExportMetricsServiceRequest,
        byte_array: &mut Vec<u8>,
        metric: &opentelemetry_sdk::metrics::data::Metric,
        sum: &opentelemetry_sdk::metrics::data::Sum<T>,
    ) -> usize {
        let start_time = to_nanos(sum.start_time());
        let time = to_nanos(sum.time());
        let is_monotonic = sum.is_monotonic();
        let temporality = sum.temporality() as i32;
        let default_flags =
            opentelemetry_proto::tonic::metrics::v1::DataPointFlags::default() as u32;

        Self::set_metric_shell(request, metric);

        let points = sum.data_points().map(|dp| {
            opentelemetry_proto::tonic::metrics::v1::NumberDataPoint {
                attributes: dp.attributes().map(Into::into).collect(),
                start_time_unix_nano: start_time,
                time_unix_nano: time,
                exemplars: Vec::new(), // No support for exemplars
                flags: default_flags,
                value: Some(dp.value().into_number_data_point_value()),
            }
        });

        self.emit_batched(request, byte_array, metric, points, |data_points| {
            ProtoData::Sum(opentelemetry_proto::tonic::metrics::v1::Sum {
                aggregation_temporality: temporality,
                is_monotonic,
                data_points,
            })
        })
    }

    fn process_histogram<T: Numeric>(
        &self,
        request: &mut ExportMetricsServiceRequest,
        byte_array: &mut Vec<u8>,
        metric: &opentelemetry_sdk::metrics::data::Metric,
        hist: &opentelemetry_sdk::metrics::data::Histogram<T>,
    ) -> usize {
        let start_time = to_nanos(hist.start_time());
        let time = to_nanos(hist.time());
        let temporality = hist.temporality() as i32;
        let default_flags =
            opentelemetry_proto::tonic::metrics::v1::DataPointFlags::default() as u32;

        Self::set_metric_shell(request, metric);

        let points = hist.data_points().map(|dp| {
            opentelemetry_proto::tonic::metrics::v1::HistogramDataPoint {
                attributes: dp.attributes().map(Into::into).collect(),
                start_time_unix_nano: start_time,
                time_unix_nano: time,
                count: dp.count(),
                sum: Some(dp.sum().into_f64()),
                bucket_counts: dp.bucket_counts().collect(),
                explicit_bounds: dp.bounds().collect(),
                exemplars: Vec::new(), // No support for exemplars
                flags: default_flags,
                min: dp.min().map(|v| v.into_f64()),
                max: dp.max().map(|v| v.into_f64()),
            }
        });

        self.emit_batched(request, byte_array, metric, points, |data_points| {
            ProtoData::Histogram(opentelemetry_proto::tonic::metrics::v1::Histogram {
                aggregation_temporality: temporality,
                data_points,
            })
        })
    }

    fn process_exponential_histogram<T: Numeric>(
        &self,
        request: &mut ExportMetricsServiceRequest,
        byte_array: &mut Vec<u8>,
        metric: &opentelemetry_sdk::metrics::data::Metric,
        hist: &opentelemetry_sdk::metrics::data::ExponentialHistogram<T>,
    ) -> usize {
        let start_time = to_nanos(hist.start_time());
        let time = to_nanos(hist.time());
        let temporality = hist.temporality() as i32;
        let default_flags =
            opentelemetry_proto::tonic::metrics::v1::DataPointFlags::default() as u32;

        Self::set_metric_shell(request, metric);

        let points = hist.data_points().map(|dp| {
            opentelemetry_proto::tonic::metrics::v1::ExponentialHistogramDataPoint {
                attributes: dp.attributes().map(Into::into).collect(),
                start_time_unix_nano: start_time,
                time_unix_nano: time,
                count: dp.count() as u64,
                sum: Some(dp.sum().into_f64()),
                scale: dp.scale().into(),
                zero_count: dp.zero_count(),
                positive: Some(
                    opentelemetry_proto::tonic::metrics::v1::exponential_histogram_data_point::Buckets {
                        offset: dp.positive_bucket().offset(),
                        bucket_counts: dp.positive_bucket().counts().collect(),
                    },
                ),
                negative: Some(
                    opentelemetry_proto::tonic::metrics::v1::exponential_histogram_data_point::Buckets {
                        offset: dp.negative_bucket().offset(),
                        bucket_counts: dp.negative_bucket().counts().collect(),
                    },
                ),
                exemplars: Vec::new(), // No support for exemplars
                flags: default_flags,
                min: dp.min().map(|v| v.into_f64()),
                max: dp.max().map(|v| v.into_f64()),
                zero_threshold: dp.zero_threshold(),
            }
        });

        self.emit_batched(request, byte_array, metric, points, |data_points| {
            ProtoData::ExponentialHistogram(
                opentelemetry_proto::tonic::metrics::v1::ExponentialHistogram {
                    aggregation_temporality: temporality,
                    data_points,
                },
            )
        })
    }

    fn encode_and_emit_metric(
        &self,
        request: &ExportMetricsServiceRequest,
        byte_array: &mut Vec<u8>,
        metric: &opentelemetry_sdk::metrics::data::Metric,
    ) -> Result<(), String> {
        match request.encode(byte_array) {
            Ok(_) => {
                otel_debug!(name: "SerializationSucceeded",
                    metric_name = metric.name(),
                    size = byte_array.len());

                if byte_array.len() > self.max_event_size {
                    let error_msg = format!(
                        "Encoded event size exceeds maximum allowed limit of {} bytes. Event will be dropped.",
                        self.max_event_size
                    );
                    otel_debug!(
                        name: "EventSizeExceeded",
                        reason = &error_msg,
                        metric_name = metric.name(),
                        size = byte_array.len()
                    );
                    Err(error_msg)
                } else {
                    // Write to the tracepoint
                    let result = self.writer.write(byte_array);
                    if result == 0 {
                        otel_debug!(name: "TracepointWriteSucceeded", message = "Encoded data successfully written to tracepoint", size = byte_array.len(), metric_name = metric.name());
                        Ok(())
                    } else {
                        let error_msg = "Failed to write to tracepoint".to_string();
                        otel_debug!(name: "TracepointWriteFailed", message = &error_msg, metric_name = metric.name(), result = result);
                        Err(error_msg)
                    }
                }
            }
            Err(err) => {
                let error_msg = format!("Serialization failed: {err}");
                otel_debug!(name: "SerializationFailed",
                    error = &error_msg,
                    metric_name = metric.name(),
                    size = byte_array.len());
                Err(error_msg)
            }
        }
    }

    fn export_resource_metrics(&self, resource_metric: &ResourceMetrics) -> OTelSdkResult {
        // Custom transformation to protobuf structs is used instead of upstream
        // transforms because the tracepoint has a 64kB size limit. Data points
        // are packed into as few events as possible while respecting that limit,
        // so the resource/scope/metric envelope is amortized across many data
        // points instead of being repeated for every one.
        //
        // Batching is currently per-metric: a single event never mixes data
        // points from different metrics or scopes. Packing across metrics would
        // amortize the envelope further and is left as a follow-up.
        let mut byte_array = Vec::new();
        let mut has_failures = false;
        let mut request = ExportMetricsServiceRequest {
            resource_metrics: vec![opentelemetry_proto::tonic::metrics::v1::ResourceMetrics {
                resource: Some((resource_metric.resource()).into()),
                scope_metrics: vec![],
                schema_url: resource_metric
                    .resource()
                    .schema_url()
                    .unwrap_or_default()
                    .to_string(),
            }],
        };

        for scope_metric in resource_metric.scope_metrics() {
            request.resource_metrics[0].scope_metrics =
                vec![opentelemetry_proto::tonic::metrics::v1::ScopeMetrics {
                    scope: Some((scope_metric.scope(), None).into()),
                    metrics: vec![],
                    schema_url: scope_metric
                        .scope()
                        .schema_url()
                        .unwrap_or_default()
                        .to_string(),
                }];

            for metric in scope_metric.metrics() {
                let failed_count = match metric.data() {
                    AggregatedMetrics::F64(data) => {
                        self.process_numeric_metrics(&mut request, &mut byte_array, metric, data)
                    }
                    AggregatedMetrics::U64(data) => {
                        self.process_numeric_metrics(&mut request, &mut byte_array, metric, data)
                    }
                    AggregatedMetrics::I64(data) => {
                        self.process_numeric_metrics(&mut request, &mut byte_array, metric, data)
                    }
                };

                // Log failure counts if any data points failed to export
                if failed_count > 0 {
                    has_failures = true;
                }
            }
        }

        // Even a single failure in the export process is considered a failure of overall export
        // The debug level logs will show exactly which metrics failed
        if has_failures {
            Err(OTelSdkError::InternalFailure(
                "Failed to export some metrics due to serialization or tracepoint write errors"
                    .to_string(),
            ))
        } else {
            Ok(())
        }
    }
}

impl PushMetricExporter for MetricsExporter {
    async fn export(&self, resource_metrics: &ResourceMetrics) -> OTelSdkResult {
        otel_debug!(name: "ExportStarted", message = "Starting metrics export");
        if !self.writer.enabled() {
            // TODO - This can flood the logs if the tracepoint is disabled for long periods of time
            otel_info!(name: "TracepointDisabled", message = "Tracepoint is disabled, skipping export");
            Ok(())
        } else {
            self.export_resource_metrics(resource_metrics)
        }
    }

    fn temporality(&self) -> Temporality {
        Temporality::Delta
    }

    fn force_flush(&self) -> OTelSdkResult {
        Ok(()) // In this implementation, flush does nothing
    }

    fn shutdown_with_timeout(&self, _timeout: std::time::Duration) -> OTelSdkResult {
        // TracepointState automatically deregisters when dropped
        // https://github.com/microsoft/LinuxTracepoints-Rust/blob/main/eventheader/src/native.rs#L618
        Ok(())
    }

    fn shutdown(&self) -> OTelSdkResult {
        self.shutdown_with_timeout(Duration::from_secs(5))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::metrics::MeterProvider;
    use opentelemetry::KeyValue;
    use opentelemetry_proto::tonic::metrics::v1::metric::Data as PData;
    use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
    use opentelemetry_sdk::Resource;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct CollectingWriter {
        events: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl CollectingWriter {
        fn take(&self) -> Vec<Vec<u8>> {
            std::mem::take(&mut *self.events.lock().unwrap())
        }
    }

    impl EventWriter for CollectingWriter {
        fn enabled(&self) -> bool {
            true
        }

        fn write(&self, buffer: &[u8]) -> i32 {
            self.events.lock().unwrap().push(buffer.to_vec());
            0
        }
    }

    impl MetricsExporter {
        fn for_test(writer: CollectingWriter, max_event_size: usize) -> Self {
            MetricsExporter {
                writer: Box::new(writer),
                max_event_size,
            }
        }
    }

    /// Replaces the data points of `request` with `data_points`, preserving the
    /// metric type and its type-specific fields.
    fn with_data_points(
        request: &ExportMetricsServiceRequest,
        indices: &[usize],
    ) -> ExportMetricsServiceRequest {
        let mut out = request.clone();
        let data = out.resource_metrics[0].scope_metrics[0].metrics[0]
            .data
            .as_mut()
            .expect("metric has data");
        fn keep<T: Clone>(points: &mut Vec<T>, indices: &[usize]) {
            let all = std::mem::take(points);
            *points = indices.iter().map(|&i| all[i].clone()).collect();
        }
        match data {
            PData::Gauge(g) => keep(&mut g.data_points, indices),
            PData::Sum(s) => keep(&mut s.data_points, indices),
            PData::Histogram(h) => keep(&mut h.data_points, indices),
            PData::ExponentialHistogram(h) => keep(&mut h.data_points, indices),
            PData::Summary(s) => keep(&mut s.data_points, indices),
        }
        out
    }

    /// Encoded cost of the first data point of `request`, as charged by the
    /// batching accounting in the exporter.
    fn first_point_cost(request: &ExportMetricsServiceRequest) -> usize {
        match request.resource_metrics[0].scope_metrics[0].metrics[0]
            .data
            .as_ref()
            .expect("metric has data")
        {
            PData::Gauge(g) => data_point_cost(&g.data_points[0]),
            PData::Sum(s) => data_point_cost(&s.data_points[0]),
            PData::Histogram(h) => data_point_cost(&h.data_points[0]),
            PData::ExponentialHistogram(h) => data_point_cost(&h.data_points[0]),
            PData::Summary(s) => data_point_cost(&s.data_points[0]),
        }
    }

    fn data_point_count(request: &ExportMetricsServiceRequest) -> usize {
        match request.resource_metrics[0].scope_metrics[0].metrics[0]
            .data
            .as_ref()
            .expect("metric has data")
        {
            PData::Gauge(g) => g.data_points.len(),
            PData::Sum(s) => s.data_points.len(),
            PData::Histogram(h) => h.data_points.len(),
            PData::ExponentialHistogram(h) => h.data_points.len(),
            PData::Summary(s) => s.data_points.len(),
        }
    }

    #[derive(Debug)]
    struct Stats {
        batched_events: usize,
        batched_bytes: usize,
        baseline_events: usize,
        baseline_bytes: usize,
        data_points: usize,
        envelope_bytes: usize,
    }

    impl Stats {
        fn byte_ratio(&self) -> f64 {
            self.baseline_bytes as f64 / self.batched_bytes as f64
        }
        fn write_ratio(&self) -> f64 {
            self.baseline_events as f64 / self.batched_events as f64
        }
        /// Share of the unbatched payload that is pure repeated envelope.
        fn baseline_waste_pct(&self) -> f64 {
            (self.envelope_bytes * self.baseline_events) as f64 / self.baseline_bytes as f64 * 100.0
        }
    }

    /// Measures what the exporter actually wrote, and what the same data would
    /// have cost under the previous one-event-per-data-point scheme.
    fn measure(events: &[Vec<u8>]) -> Stats {
        let mut stats = Stats {
            batched_events: events.len(),
            batched_bytes: events.iter().map(|e| e.len()).sum(),
            baseline_events: 0,
            baseline_bytes: 0,
            data_points: 0,
            envelope_bytes: 0,
        };

        for raw in events {
            let request =
                ExportMetricsServiceRequest::decode(raw.as_slice()).expect("decodes as OTLP");
            let count = data_point_count(&request);
            stats.data_points += count;
            stats.baseline_events += count;
            for i in 0..count {
                stats.baseline_bytes += with_data_points(&request, &[i]).encoded_len();
            }
            // Envelope = the same event with zero data points. Recorded from the
            // first event only; it is identical across events of a metric.
            if stats.envelope_bytes == 0 {
                stats.envelope_bytes = with_data_points(&request, &[]).encoded_len();
            }
        }

        stats
    }

    fn report(label: &str, stats: &Stats, expected_dps: usize) {
        assert_eq!(
            stats.data_points, expected_dps,
            "{label}: scenario did not produce the expected number of series"
        );
        println!(
            "{label:<44} | {:>7} | {:>6} | {:>10} | {:>10} | {:>5.2}x | {:>5.0}x | {:>5.1}%",
            stats.data_points,
            stats.envelope_bytes,
            stats.baseline_bytes,
            stats.batched_bytes,
            stats.byte_ratio(),
            stats.write_ratio(),
            stats.baseline_waste_pct(),
        );
    }

    fn header() {
        println!(
            "\n{:<44} | {:>7} | {:>6} | {:>10} | {:>10} | {:>6} | {:>6} | {:>6}",
            "scenario", "dps", "envlp", "unbatched", "batched", "bytes", "writes", "waste"
        );
        println!("{}", "-".repeat(120));
    }

    /// Runs `record` against a meter provider wired to a collecting exporter and
    /// returns the raw events the exporter produced.
    fn run_scenario<F>(max_event_size: usize, record: F) -> Vec<Vec<u8>>
    where
        F: FnOnce(&opentelemetry::metrics::Meter),
    {
        let writer = CollectingWriter::default();
        let exporter = MetricsExporter::for_test(writer.clone(), max_event_size);
        let reader = PeriodicReader::builder(exporter).build();
        let provider = SdkMeterProvider::builder()
            .with_reader(reader)
            .with_resource(
                Resource::builder_empty()
                    .with_attributes([
                        KeyValue::new("service.name", "contoso-ingest"),
                        KeyValue::new(
                            "service.instance.id",
                            "9f1c2b7e-4a3d-4c11-9b02-7e5f8a1d3c44",
                        ),
                        KeyValue::new("cloud.region", "eastus2"),
                        KeyValue::new("host.name", "node-prod-0731"),
                    ])
                    .build(),
            )
            .build();

        record(&provider.meter("contoso.ingest.meter"));
        // Flush may legitimately report failure when a scenario deliberately
        // configures a size limit that some data point cannot satisfy; callers
        // assert on the emitted events instead.
        let _ = provider.force_flush();
        // Collect before shutdown: shutdown performs another collection, which
        // would invoke observable callbacks a second time and double-count.
        let events = writer.take();
        let _ = provider.shutdown();
        events
    }

    /// Attribute sets for `count` dimensions. The high-cardinality `partition`
    /// key comes first so that every dimension count still yields distinct
    /// series.
    fn dims(i: usize, count: usize) -> Vec<KeyValue> {
        let all = [
            KeyValue::new("partition", format!("p{i:04}")),
            KeyValue::new("operation", "GetBlobProperties"),
            KeyValue::new("status_code", "200"),
            KeyValue::new("client_version", "2024-11-04"),
            KeyValue::new("protocol", "https"),
        ];
        all.into_iter().take(count).collect()
    }

    const SERIES: usize = 1000;

    #[test]
    fn batching_savings_report() {
        header();

        // Counter, 5 dimensions.
        let events = run_scenario(MAX_EVENT_SIZE, |meter| {
            let c = meter.u64_counter("contoso.ingest.requests").build();
            for i in 0..SERIES {
                c.add(1, &dims(i, 5));
            }
        });
        let stats = measure(&events);
        report("counter, 5 dims", &stats, SERIES);

        // Counter, 2 dimensions: smaller payload against the same envelope, so
        // the relative waste is worse.
        let events = run_scenario(MAX_EVENT_SIZE, |meter| {
            let c = meter.u64_counter("contoso.ingest.requests").build();
            for i in 0..SERIES {
                c.add(1, &dims(i, 2));
            }
        });
        let stats = measure(&events);
        report("counter, 2 dims", &stats, SERIES);

        // Observable gauge.
        let events = run_scenario(MAX_EVENT_SIZE, |meter| {
            meter
                .u64_observable_gauge("contoso.ingest.queue_depth")
                .with_callback(|obs| {
                    for i in 0..SERIES {
                        obs.observe(i as u64, &dims(i, 4));
                    }
                })
                .build();
        });
        let stats = measure(&events);
        report("observable gauge, 4 dims", &stats, SERIES);

        // Histogram: much larger data points (bucket counts + bounds), so the
        // envelope is proportionally less significant.
        let events = run_scenario(MAX_EVENT_SIZE, |meter| {
            let h = meter.f64_histogram("contoso.ingest.latency").build();
            for i in 0..SERIES {
                h.record(12.5, &dims(i, 3));
            }
        });
        let stats = measure(&events);
        report("histogram, 3 dims", &stats, SERIES);

        println!();
    }

    #[test]
    fn every_event_is_within_the_size_limit() {
        for max in [MAX_EVENT_SIZE, 8192, 2048, 512] {
            let events = run_scenario(max, |meter| {
                let c = meter.u64_counter("contoso.ingest.requests").build();
                for i in 0..SERIES {
                    c.add(1, &dims(i, 5));
                }
            });
            assert!(!events.is_empty(), "max={max} produced no events");
            for event in &events {
                assert!(
                    event.len() <= max,
                    "event of {} bytes exceeds max_event_size {max}",
                    event.len()
                );
            }
            let stats = measure(&events);
            assert_eq!(
                stats.data_points, SERIES,
                "max={max} lost or duplicated data points"
            );

            // Every event except the last must have been flushed because the
            // next data point genuinely did not fit, not because the size
            // accounting gave up early.
            for i in 0..events.len().saturating_sub(1) {
                let next = ExportMetricsServiceRequest::decode(events[i + 1].as_slice()).unwrap();
                let next_cost = first_point_cost(&next);
                let remaining = max - events[i].len();
                assert!(
                    remaining < next_cost + SIZE_SLACK,
                    "event left {remaining} bytes unused but the next data point \
                     needs only {next_cost} (max={max}); accounting is too conservative"
                );
            }
        }
    }

    #[test]
    fn batching_preserves_every_data_point_exactly_once() {
        let events = run_scenario(4096, |meter| {
            let c = meter.u64_counter("contoso.ingest.requests").build();
            for i in 0..SERIES {
                c.add(i as u64, &dims(i, 5));
            }
        });

        let mut seen: Vec<(String, u64)> = Vec::new();
        for raw in &events {
            let request = ExportMetricsServiceRequest::decode(raw.as_slice()).unwrap();
            let PData::Sum(sum) = request.resource_metrics[0].scope_metrics[0].metrics[0]
                .data
                .as_ref()
                .unwrap()
            else {
                panic!("counter should encode as Sum");
            };
            for dp in &sum.data_points {
                let partition = dp
                    .attributes
                    .iter()
                    .find(|kv| kv.key == "partition")
                    .and_then(|kv| kv.value.as_ref())
                    .and_then(|v| match v.value.as_ref() {
                        Some(
                            opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                                s,
                            ),
                        ) => Some(s.clone()),
                        _ => None,
                    })
                    .expect("partition attribute present");
                let value = match dp.value.as_ref().unwrap() {
                    opentelemetry_proto::tonic::metrics::v1::number_data_point::Value::AsInt(v) => {
                        *v as u64
                    }
                    _ => panic!("counter should encode as int"),
                };
                seen.push((partition, value));
            }
        }

        assert_eq!(seen.len(), SERIES, "wrong number of data points");
        seen.sort();
        seen.dedup_by(|a, b| a.0 == b.0);
        assert_eq!(seen.len(), SERIES, "duplicate series across batches");
        for (partition, value) in &seen {
            let i: usize = partition.trim_start_matches('p').parse().unwrap();
            assert_eq!(*value, i as u64, "value mismatch for {partition}");
        }
    }

    #[test]
    fn metric_metadata_is_repeated_in_every_event() {
        // Each event must be independently decodable: resource, scope and metric
        // identity are required in all of them, not just the first.
        let events = run_scenario(2048, |meter| {
            let c = meter.u64_counter("contoso.ingest.requests").build();
            for i in 0..SERIES {
                c.add(1, &dims(i, 5));
            }
        });
        assert!(
            events.len() > 1,
            "expected the batch to span several events"
        );
        for raw in &events {
            let request = ExportMetricsServiceRequest::decode(raw.as_slice()).unwrap();
            let rm = &request.resource_metrics[0];
            assert_eq!(rm.resource.as_ref().unwrap().attributes.len(), 4);
            let sm = &rm.scope_metrics[0];
            assert_eq!(sm.scope.as_ref().unwrap().name, "contoso.ingest.meter");
            assert_eq!(sm.metrics[0].name, "contoso.ingest.requests");
        }
    }

    #[test]
    fn oversized_single_data_point_is_dropped_without_panicking() {
        // A max_event_size below the envelope size means no data point can ever
        // fit. The exporter must drop them and stay alive.
        let events = run_scenario(64, |meter| {
            let c = meter.u64_counter("contoso.ingest.requests").build();
            for i in 0..10 {
                c.add(1, &dims(i, 5));
            }
        });
        assert!(
            events.is_empty(),
            "nothing should be written when no data point fits"
        );
    }

    #[test]
    fn varint_len_matches_prost() {
        for len in [0usize, 1, 127, 128, 16_383, 16_384, 65_360, 2_097_151] {
            let mut buf = Vec::new();
            prost::encoding::encode_varint(len as u64, &mut buf);
            assert_eq!(varint_len(len), buf.len(), "varint_len({len})");
        }
    }
}
