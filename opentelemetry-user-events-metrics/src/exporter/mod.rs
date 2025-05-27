use opentelemetry::{otel_debug, otel_info};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
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
    trace_point: Pin<Box<ehi::TracepointState>>,
}

impl MetricsExporter {
    pub fn new() -> MetricsExporter {
        let trace_point = Box::pin(ehi::TracepointState::new(0));
        // This is unsafe because if the code is used in a shared object,
        // the event MUST be unregistered before the shared object unloads.
        unsafe {
            let _result = tracepoint::register(trace_point.as_ref());
        }
        MetricsExporter { trace_point }
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

pub(crate) fn to_nanos(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_nanos() as u64
}

impl MetricsExporter {
    fn process_numeric_metrics<T: Numeric>(
        &self,
        export_metric_service_request_common: &mut ExportMetricsServiceRequest,
        byte_array: &mut Vec<u8>,
        metric: &opentelemetry_sdk::metrics::data::Metric,
        data: &MetricData<T>,
    ) -> usize {
        match data {
            MetricData::Gauge(gauge) => self.process_gauge(
                export_metric_service_request_common,
                byte_array,
                metric,
                gauge,
            ),
            MetricData::Sum(sum) => self.process_sum(
                export_metric_service_request_common,
                byte_array,
                metric,
                sum,
            ),
            MetricData::Histogram(hist) => self.process_histogram(
                export_metric_service_request_common,
                byte_array,
                metric,
                hist,
            ),
            MetricData::ExponentialHistogram(hist) => self.process_exponential_histogram(
                export_metric_service_request_common,
                byte_array,
                metric,
                hist,
            ),
        }
    }

    fn process_gauge<T: Numeric>(
        &self,
        export_metric_service_request_common: &mut ExportMetricsServiceRequest,
        byte_array: &mut Vec<u8>,
        metric: &opentelemetry_sdk::metrics::data::Metric,
        gauge: &opentelemetry_sdk::metrics::data::Gauge<T>,
    ) -> usize {
        // Store and reuse common values for all data points in this gauge
        let gauge_start_time = gauge.start_time().map(to_nanos).unwrap_or_default();
        let gauge_time = to_nanos(gauge.time());
        let default_flags =
            opentelemetry_proto::tonic::metrics::v1::DataPointFlags::default() as u32;

        // Create metric_proto template outside the loop to reuse name, description, unit
        let metric_proto = opentelemetry_proto::tonic::metrics::v1::Metric {
            name: metric.name().to_string(),
            description: metric.description().to_string(),
            unit: metric.unit().to_string(),
            metadata: vec![],
            data: None,
        };

        export_metric_service_request_common.resource_metrics[0].scope_metrics[0].metrics =
            vec![metric_proto];

        let mut failed_count = 0;

        // ═══════════════════════════════════════════════════════════════════════════════════
        // LOOP 3: METRIC → DATA POINTS (Individual measurements)
        // ═══════════════════════════════════════════════════════════════════════════════════
        // Iterate through each data point within this gauge metric.
        // Each data point = unique combination of (metric + attributes + timestamp + value).
        // SERIALIZATION: Each data point is individually encoded and emitted to tracepoint.
        for dp in gauge.data_points() {
            let number_data_point = opentelemetry_proto::tonic::metrics::v1::NumberDataPoint {
                attributes: dp.attributes().map(Into::into).collect(),
                start_time_unix_nano: gauge_start_time,
                time_unix_nano: gauge_time,
                exemplars: Vec::new(), // No support for exemplars
                flags: default_flags,
                value: Some(dp.value().into_number_data_point_value()),
            };

            let gauge_point_proto = opentelemetry_proto::tonic::metrics::v1::Gauge {
                data_points: vec![number_data_point],
            };

            // Update the data field for this data point
            export_metric_service_request_common.resource_metrics[0].scope_metrics[0].metrics[0]
                .data = Some(
                opentelemetry_proto::tonic::metrics::v1::metric::Data::Gauge(gauge_point_proto),
            );

            byte_array.clear(); // Clear contents but retain capacity for performance
            if self.encode_and_emit_metric(
                export_metric_service_request_common,
                byte_array,
                metric,
            ).is_err() {
                failed_count += 1;
            }
        }
        failed_count
    }

    fn process_sum<T: Numeric>(
        &self,
        export_metric_service_request_common: &mut ExportMetricsServiceRequest,
        byte_array: &mut Vec<u8>,
        metric: &opentelemetry_sdk::metrics::data::Metric,
        sum: &opentelemetry_sdk::metrics::data::Sum<T>,
    ) -> usize {
        // Pre-compute common values for all data points in this sum
        let sum_start_time = to_nanos(sum.start_time());
        let sum_time = to_nanos(sum.time());
        let sum_is_monotonic = sum.is_monotonic();
        let default_flags =
            opentelemetry_proto::tonic::metrics::v1::DataPointFlags::default() as u32;

        // Create metric_proto template outside the loop to reuse name, description, unit
        let metric_proto = opentelemetry_proto::tonic::metrics::v1::Metric {
            name: metric.name().to_string(),
            description: metric.description().to_string(),
            unit: metric.unit().to_string(),
            metadata: vec![],
            data: None,
        };

        export_metric_service_request_common.resource_metrics[0].scope_metrics[0].metrics =
            vec![metric_proto];

        let mut failed_count = 0;

        // ═══════════════════════════════════════════════════════════════════════════════════
        // LOOP 3: METRIC → DATA POINTS (Individual measurements)
        // ═══════════════════════════════════════════════════════════════════════════════════
        // Iterate through each data point within this sum metric.
        // Each data point = unique combination of (metric + attributes + timestamp + value).
        // SERIALIZATION: Each data point is individually encoded and emitted to tracepoint.
        for dp in sum.data_points() {
            let number_data_point = opentelemetry_proto::tonic::metrics::v1::NumberDataPoint {
                attributes: dp.attributes().map(Into::into).collect(),
                start_time_unix_nano: sum_start_time,
                time_unix_nano: sum_time,
                exemplars: Vec::new(), // No support for exemplars
                flags: default_flags,
                value: Some(dp.value().into_number_data_point_value()),
            };

            let sum_point_proto = opentelemetry_proto::tonic::metrics::v1::Sum {
                aggregation_temporality: 1,
                is_monotonic: sum_is_monotonic,
                data_points: vec![number_data_point],
            };

            // Update the data field for this data point
            export_metric_service_request_common.resource_metrics[0].scope_metrics[0].metrics[0]
                .data = Some(opentelemetry_proto::tonic::metrics::v1::metric::Data::Sum(
                sum_point_proto,
            ));

            byte_array.clear(); // Clear contents but retain capacity for performance
            if self.encode_and_emit_metric(
                export_metric_service_request_common,
                byte_array,
                metric,
            ).is_err() {
                failed_count += 1;
            }
        }
        failed_count
    }

    fn process_histogram<T: Numeric>(
        &self,
        export_metric_service_request_common: &mut ExportMetricsServiceRequest,
        byte_array: &mut Vec<u8>,
        metric: &opentelemetry_sdk::metrics::data::Metric,
        hist: &opentelemetry_sdk::metrics::data::Histogram<T>,
    ) -> usize {
        // Pre-compute common values for all data points in this histogram
        let hist_start_time = to_nanos(hist.start_time());
        let hist_time = to_nanos(hist.time());
        let default_flags =
            opentelemetry_proto::tonic::metrics::v1::DataPointFlags::default() as u32;

        // Create metric_proto template outside the loop to reuse name, description, unit
        let metric_proto = opentelemetry_proto::tonic::metrics::v1::Metric {
            name: metric.name().to_string(),
            description: metric.description().to_string(),
            unit: metric.unit().to_string(),
            metadata: vec![],
            data: None,
        };

        export_metric_service_request_common.resource_metrics[0].scope_metrics[0].metrics =
            vec![metric_proto];

        let mut failed_count = 0;

        // ═══════════════════════════════════════════════════════════════════════════════════
        // LOOP 3: METRIC → DATA POINTS (Individual measurements)
        // ═══════════════════════════════════════════════════════════════════════════════════
        // Iterate through each data point within this histogram metric.
        // Each data point = unique combination of (metric + attributes + timestamp + buckets).
        // SERIALIZATION: Each data point is individually encoded and emitted to tracepoint.
        for dp in hist.data_points() {
            let histogram_data_point =
                opentelemetry_proto::tonic::metrics::v1::HistogramDataPoint {
                    attributes: dp.attributes().map(Into::into).collect(),
                    start_time_unix_nano: hist_start_time,
                    time_unix_nano: hist_time,
                    count: dp.count(),
                    sum: Some(dp.sum().into_f64()),
                    bucket_counts: dp.bucket_counts().collect(),
                    explicit_bounds: dp.bounds().collect(),
                    exemplars: Vec::new(), // No support for exemplars
                    flags: default_flags,
                    min: dp.min().map(|v| v.into_f64()),
                    max: dp.max().map(|v| v.into_f64()),
                };

            let histogram_point_proto = opentelemetry_proto::tonic::metrics::v1::Histogram {
                aggregation_temporality: 1,
                data_points: vec![histogram_data_point],
            };

            // Update the data field for this data point
            export_metric_service_request_common.resource_metrics[0].scope_metrics[0].metrics[0]
                .data = Some(
                opentelemetry_proto::tonic::metrics::v1::metric::Data::Histogram(
                    histogram_point_proto,
                ),
            );

            byte_array.clear(); // Clear contents but retain capacity for performance
            if self.encode_and_emit_metric(
                export_metric_service_request_common,
                byte_array,
                metric,
            ).is_err() {
                failed_count += 1;
            }
        }
        failed_count
    }

    fn process_exponential_histogram<T: Numeric>(
        &self,
        export_metric_service_request_common: &mut ExportMetricsServiceRequest,
        byte_array: &mut Vec<u8>,
        metric: &opentelemetry_sdk::metrics::data::Metric,
        hist: &opentelemetry_sdk::metrics::data::ExponentialHistogram<T>,
    ) -> usize {
        // Pre-compute common values for all data points in this histogram
        let hist_start_time = to_nanos(hist.start_time());
        let hist_time = to_nanos(hist.time());
        let default_flags =
            opentelemetry_proto::tonic::metrics::v1::DataPointFlags::default() as u32;

        // Create metric_proto template outside the loop to reuse name, description, unit
        let metric_proto = opentelemetry_proto::tonic::metrics::v1::Metric {
            name: metric.name().to_string(),
            description: metric.description().to_string(),
            unit: metric.unit().to_string(),
            metadata: vec![],
            data: None,
        };

        export_metric_service_request_common.resource_metrics[0].scope_metrics[0].metrics =
            vec![metric_proto];

        let mut failed_count = 0;

        // ═══════════════════════════════════════════════════════════════════════════════════
        // LOOP 3: METRIC → DATA POINTS (Individual measurements)
        // ═══════════════════════════════════════════════════════════════════════════════════
        // Iterate through each data point within this exponential histogram metric.
        // Each data point = unique combination of (metric + attributes + timestamp + buckets).
        // SERIALIZATION: Each data point is individually encoded and emitted to tracepoint.
        for dp in hist.data_points() {
            let histogram_data_point = opentelemetry_proto::tonic::metrics::v1::ExponentialHistogramDataPoint {
                attributes: dp.attributes().map(Into::into).collect(),
                start_time_unix_nano: hist_start_time,
                time_unix_nano: hist_time,
                count: dp.count() as u64,
                sum: Some(dp.sum().into_f64()),
                scale: dp.scale().into(),
                zero_count: dp.zero_count(),
                positive: Some(opentelemetry_proto::tonic::metrics::v1::exponential_histogram_data_point::Buckets {
                    offset: dp.positive_bucket().offset(),
                    bucket_counts: dp.positive_bucket().counts().collect(),
                }),
                negative: Some(opentelemetry_proto::tonic::metrics::v1::exponential_histogram_data_point::Buckets {
                    offset: dp.negative_bucket().offset(),
                    bucket_counts: dp.negative_bucket().counts().collect(),
                }),
                exemplars: Vec::new(), // No support for exemplars
                flags: default_flags,
                min: dp.min().map(|v| v.into_f64()),
                max: dp.max().map(|v| v.into_f64()),
                zero_threshold: dp.zero_threshold(),
            };

            let histogram_point_proto =
                opentelemetry_proto::tonic::metrics::v1::ExponentialHistogram {
                    aggregation_temporality: 1,
                    data_points: vec![histogram_data_point],
                };

            // Update the data field for this data point
            export_metric_service_request_common.resource_metrics[0].scope_metrics[0].metrics[0]
                .data = Some(
                opentelemetry_proto::tonic::metrics::v1::metric::Data::ExponentialHistogram(
                    histogram_point_proto,
                ),
            );

            byte_array.clear(); // Clear contents but retain capacity for performance
            if self.encode_and_emit_metric(
                export_metric_service_request_common,
                byte_array,
                metric,
            ).is_err() {
                failed_count += 1;
            }
        }
        failed_count
    }

    fn encode_and_emit_metric(
        &self,
        export_metric_service_request_common: &ExportMetricsServiceRequest,
        byte_array: &mut Vec<u8>,
        metric: &opentelemetry_sdk::metrics::data::Metric,
    ) -> Result<(), String> {
        match export_metric_service_request_common.encode(byte_array) {
            Ok(_) => {
                otel_debug!(name: "SerializeSuccess", 
                    metric_name = metric.name(),
                    size = byte_array.len());

                if byte_array.len() > MAX_EVENT_SIZE {
                    let error_msg = format!("Encoded event size exceeds maximum allowed limit of {} bytes. Event will be dropped.", MAX_EVENT_SIZE);
                    otel_debug!(
                        name: "MaxEventSizeExceeded",
                        reason = &error_msg,
                        metric_name = metric.name(),
                        size = byte_array.len()
                    );
                    Err(error_msg)
                } else {
                    // Write to the tracepoint
                    let result = tracepoint::write(&self.trace_point, byte_array);
                    if result == 0 {
                        otel_debug!(name: "TracepointWrite", message = "Encoded data successfully written to tracepoint", size = byte_array.len(), metric_name = metric.name());
                        Ok(())
                    } else {
                        let error_msg = "Failed to write to tracepoint".to_string();
                        otel_debug!(name: "TracepointWriteFailed", message = &error_msg, metric_name = metric.name(), result = result);
                        Err(error_msg)
                    }
                }
            }
            Err(err) => {
                let error_msg = format!("Serialization failed: {}", err);
                otel_debug!(name: "SerializeFailed",
                    error = &error_msg,
                    metric_name = metric.name(),
                    size = byte_array.len());
                Err(error_msg)
            }
        }
    }

    fn export_resource_metrics(&self, resource_metric: &ResourceMetrics) -> OTelSdkResult {
        // Custom transformation to protobuf structs is used instead of upstream
        // transforms because tracepoint has a 64kB size limit. Encoding each
        // data point separately ensures we stay within this limit and avoid
        // data loss. Some upstream transforms are reused where appropriate for
        // consistency. TODO: Optimize by batching multiple data points until
        // the size limit is reached, rather than writing one data point at a
        // time.

        // OVERALL EXPORT FLOW: This method implements a 3-level nested loop
        // structure:
        //
        // LOOP 1: Resource → Scopes (Instrumentation Libraries)
        //   - Iterate through each scope/instrumentation library in the
        //     resource
        //   - Each scope contains multiple metrics from the same library
        //
        // LOOP 2: Scope → Metrics
        //   - For each scope, iterate through all metrics (counters, gauges,
        //     histograms)
        //   - Each metric contains multiple data points with different
        //     attribute combinations
        //
        // LOOP 3: Metric → Data Points
        //   - For each metric, iterate through individual data points
        //   - Each data point represents a unique combination of metric +
        //     attributes + timestamp
        //   - SERIALIZATION HAPPENS HERE: Each data point is encoded and
        //     emitted individually
        //
        // PERFORMANCE OPTIMIZATIONS:
        // - Reuse protobuf structure templates at each level to avoid repeated
        //   allocations
        // - Single byte_array is reused throughout: cleared between uses but
        //   capacity retained
        // - Common values (timestamps, flags, metadata) pre-computed (but not
        //   pre-serialized) per metric to avoid duplication
        // - Only the innermost data varies between serializations, parent
        //   structures stay constant
        let mut byte_array = Vec::new();
        let mut has_failures = false;
        let mut export_metric_service_request_common = ExportMetricsServiceRequest {
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

        // ═══════════════════════════════════════════════════════════════════════════════════
        // LOOP 1: RESOURCE → SCOPES (Instrumentation Libraries)
        // ═══════════════════════════════════════════════════════════════════════════════════
        // Iterate through each scope (instrumentation library) within this resource.
        // Each scope groups metrics that originate from the same library/component.
        for scope_metric in resource_metric.scope_metrics() {
            // Create reusable scope_metric_proto template with empty metrics
            let scope_metric_proto = opentelemetry_proto::tonic::metrics::v1::ScopeMetrics {
                scope: Some((scope_metric.scope(), None).into()),
                metrics: vec![],
                schema_url: scope_metric
                    .scope()
                    .schema_url()
                    .unwrap_or_default()
                    .to_string(),
            };

            export_metric_service_request_common.resource_metrics[0].scope_metrics =
                vec![scope_metric_proto];

            // ═══════════════════════════════════════════════════════════════════════════════════
            // LOOP 2: SCOPE → METRICS (Counters, Gauges, Histograms, etc.)
            // ═══════════════════════════════════════════════════════════════════════════════════
            // For each scope, iterate through all metrics of different types.
            // Each metric will be processed by type-specific handlers that implement LOOP 3.
            for metric in scope_metric.metrics() {
                let failed_count = match metric.data() {
                    AggregatedMetrics::F64(data) => {
                        // → DELEGATES TO LOOP 3: process_* methods iterate through data points
                        // → SERIALIZATION: Each data point encoded & emitted individually
                        // → REUSE: Same byte_array cleared & reused for performance
                        self.process_numeric_metrics(
                            &mut export_metric_service_request_common,
                            &mut byte_array,
                            metric,
                            data,
                        )
                    }
                    AggregatedMetrics::U64(data) => self.process_numeric_metrics(
                        &mut export_metric_service_request_common,
                        &mut byte_array,
                        metric,
                        data,
                    ),
                    AggregatedMetrics::I64(data) => self.process_numeric_metrics(
                        &mut export_metric_service_request_common,
                        &mut byte_array,
                        metric,
                        data,
                    ),
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
        otel_debug!(name: "ExportStart", message = "Starting metrics export");
        if !self.trace_point.enabled() {
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
