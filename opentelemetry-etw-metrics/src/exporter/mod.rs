use crate::etw;

use opentelemetry::otel_warn;
use opentelemetry_proto::tonic::{
    collector::metrics::v1::ExportMetricsServiceRequest,
    metrics::v1::{
        metric::Data as TonicMetricData, ExponentialHistogram as TonicExponentialHistogram,
        Gauge as TonicGauge, Histogram as TonicHistogram, Metric as TonicMetric,
        ResourceMetrics as TonicResourceMetrics, ScopeMetrics as TonicScopeMetrics,
        Sum as TonicSum, Summary as TonicSummary,
    },
};
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::metrics::{
    data::ResourceMetrics, exporter::PushMetricExporter, Temporality,
};

use std::fmt::{Debug, Formatter};

use prost::Message;

pub struct MetricsExporter {}

impl MetricsExporter {
    pub fn new() -> MetricsExporter {
        etw::register();

        MetricsExporter {}
    }
}

impl Default for MetricsExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for MetricsExporter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("ETW metrics exporter")
    }
}

fn emit_export_metric_service_request(
    export_metric_service_request: &ExportMetricsServiceRequest,
    encoding_buffer: &mut Vec<u8>,
) -> OTelSdkResult {
    if (export_metric_service_request.encoded_len()) > etw::MAX_EVENT_SIZE {
        otel_warn!(name: "MetricExportFailedDueToMaxSizeLimit", size = export_metric_service_request.encoded_len(), max_size = etw::MAX_EVENT_SIZE);
    } else {
        // `encoding_buffer` is assumed to be reused, so ensure it is empty before using it for encoding
        encoding_buffer.clear();

        export_metric_service_request
            .encode(encoding_buffer)
            .map_err(|err| OTelSdkError::InternalFailure(err.to_string()))?;

        let result = etw::write(encoding_buffer);
        // TODO: Better logging/internal metrics needed here for non-failure
        // case Uncomment the line below to see the exported bytes until a
        // better logging solution is implemented
        // println!("Exported {} bytes to ETW", byte_array.len());
        if result != 0 {
            otel_warn!(name: "MetricExportFailed", error_code = result);
        }
    }

    Ok(())
}

impl PushMetricExporter for MetricsExporter {
    async fn export(&self, metrics: &mut ResourceMetrics) -> OTelSdkResult {
        let schema_url: String = metrics
            .resource
            .schema_url()
            .map(Into::into)
            .unwrap_or_default();

        // TODO: Reuse this vec across exports by storing it in `MetricsExporter`
        let mut encoding_buffer = Vec::new();

        for scope_metric in &metrics.scope_metrics {
            for metric in &scope_metric.metrics {
                let proto_data: Option<TonicMetricData> = metric.data.as_any().try_into().ok();

                // This ExportMetricsServiceRequest is created for each metric and will hold a single data point.
                let mut export_metrics_service_request = ExportMetricsServiceRequest {
                    resource_metrics: vec![TonicResourceMetrics {
                        resource: Some((&metrics.resource).into()),
                        scope_metrics: vec![TonicScopeMetrics {
                            scope: Some((&scope_metric.scope, None).into()),
                            metrics: vec![TonicMetric {
                                name: metric.name.to_string(),
                                description: metric.description.to_string(),
                                unit: metric.unit.to_string(),
                                metadata: vec![],
                                data: None, // Initially data is None, it will be set based on the type of metric
                            }],
                            schema_url: schema_url.clone(),
                        }],
                        schema_url: schema_url.clone(),
                    }],
                };

                if let Some(proto_data) = proto_data {
                    match proto_data {
                        TonicMetricData::Histogram(hist) => {
                            for data_point in hist.data_points {
                                export_metrics_service_request.resource_metrics[0].scope_metrics
                                    [0]
                                .metrics[0]
                                    .data = Some(TonicMetricData::Histogram(TonicHistogram {
                                    aggregation_temporality: hist.aggregation_temporality,
                                    data_points: vec![data_point],
                                }));
                                emit_export_metric_service_request(
                                    &export_metrics_service_request,
                                    &mut encoding_buffer,
                                )?;
                            }
                        }
                        TonicMetricData::ExponentialHistogram(exp_hist) => {
                            for data_point in exp_hist.data_points {
                                export_metrics_service_request.resource_metrics[0].scope_metrics
                                    [0]
                                .metrics[0]
                                    .data = Some(TonicMetricData::ExponentialHistogram(
                                    TonicExponentialHistogram {
                                        aggregation_temporality: exp_hist.aggregation_temporality,
                                        data_points: vec![data_point],
                                    },
                                ));
                                emit_export_metric_service_request(
                                    &export_metrics_service_request,
                                    &mut encoding_buffer,
                                )?;
                            }
                        }
                        TonicMetricData::Gauge(gauge) => {
                            for data_point in gauge.data_points {
                                export_metrics_service_request.resource_metrics[0].scope_metrics
                                    [0]
                                .metrics[0]
                                    .data = Some(TonicMetricData::Gauge(TonicGauge {
                                    data_points: vec![data_point],
                                }));
                                emit_export_metric_service_request(
                                    &export_metrics_service_request,
                                    &mut encoding_buffer,
                                )?;
                            }
                        }
                        TonicMetricData::Sum(sum) => {
                            for data_point in sum.data_points {
                                export_metrics_service_request.resource_metrics[0].scope_metrics
                                    [0]
                                .metrics[0]
                                    .data = Some(TonicMetricData::Sum(TonicSum {
                                    data_points: vec![data_point],
                                    aggregation_temporality: sum.aggregation_temporality,
                                    is_monotonic: sum.is_monotonic,
                                }));
                                emit_export_metric_service_request(
                                    &export_metrics_service_request,
                                    &mut encoding_buffer,
                                )?;
                            }
                        }
                        TonicMetricData::Summary(summary) => {
                            for data in summary.data_points {
                                export_metrics_service_request.resource_metrics[0].scope_metrics
                                    [0]
                                .metrics[0]
                                    .data = Some(TonicMetricData::Summary(TonicSummary {
                                    data_points: vec![data],
                                }));
                                emit_export_metric_service_request(
                                    &export_metrics_service_request,
                                    &mut encoding_buffer,
                                )?;
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown(&self) -> OTelSdkResult {
        etw::unregister();

        Ok(())
    }

    fn temporality(&self) -> Temporality {
        Temporality::Delta
    }
}

#[cfg(test)]
mod tests {
    use opentelemetry::{metrics::MeterProvider as _, KeyValue};
    use opentelemetry_sdk::{
        metrics::{PeriodicReader, SdkMeterProvider},
        Resource,
    };

    use crate::etw;

    #[tokio::test(flavor = "multi_thread")]
    async fn emit_metrics_that_combined_exceed_etw_max_event_size() {
        let exporter = super::MetricsExporter::new();
        let reader = PeriodicReader::builder(exporter).build();
        let meter_provider = SdkMeterProvider::builder()
            .with_resource(
                Resource::builder()
                    .with_attributes(vec![KeyValue::new("service.name", "service-name")])
                    .build(),
            )
            .with_reader(reader)
            .build();

        let meter = meter_provider.meter("user-event-test");

        let u64_histogram = meter
            .u64_histogram("Testu64Histogram")
            .with_description("u64_histogram_test_description")
            .with_unit("u64_histogram_test_unit")
            .build();

        let f64_histogram = meter
            .f64_histogram("TestHistogram")
            .with_description("f64_histogram_test_description")
            .with_unit("f64_histogram_test_unit")
            .build();

        let u64_counter = meter
            .u64_counter("Testu64Counter")
            .with_description("u64_counter_test_description")
            .with_unit("u64_counter_test_units")
            .build();

        let f64_counter = meter
            .f64_counter("Testf64Counter")
            .with_description("f64_counter_test_description")
            .with_unit("f64_counter_test_units")
            .build();

        let i64_counter = meter
            .i64_up_down_counter("Testi64Counter")
            .with_description("i64_counter_test_description")
            .with_unit("i64_counter_test_units")
            .build();

        let u64_gauge = meter
            .u64_gauge("Testu64Gauge")
            .with_description("u64_gauge_test_description")
            .with_unit("u64_gauge_test_unit")
            .build();

        let i64_gauge = meter
            .i64_gauge("Testi64Gauge")
            .with_description("i64_gauge_test_description")
            .with_unit("i64_gauge_test_unit")
            .build();

        let f64_gauge = meter
            .f64_gauge("Testf64Gauge")
            .with_description("f64_gauge_test_description")
            .with_unit("f64_gauge_test_unit")
            .build();

        // Create a key that is 1/10th the size of the MAX_EVENT_SIZE
        let key_size = etw::MAX_EVENT_SIZE / 10;
        let large_key = "a".repeat(key_size);

        for index in 0..11 {
            u64_histogram.record(
                1,
                [KeyValue::new(large_key.clone(), format!("{index}"))].as_ref(),
            );
        }

        for index in 0..11 {
            f64_histogram.record(
                1.0,
                [KeyValue::new(large_key.clone(), format!("{index}"))].as_ref(),
            );
        }

        for index in 0..11 {
            u64_counter.add(
                1,
                [KeyValue::new(large_key.clone(), format!("{index}"))].as_ref(),
            );
        }

        for index in 0..11 {
            f64_counter.add(
                1.0,
                [KeyValue::new(large_key.clone(), format!("{index}"))].as_ref(),
            );
        }

        for index in 0..11 {
            i64_counter.add(
                1,
                [KeyValue::new(large_key.clone(), format!("{index}"))].as_ref(),
            );
        }

        for index in 0..11 {
            u64_gauge.record(
                1,
                [KeyValue::new(large_key.clone(), format!("{index}"))].as_ref(),
            );
        }

        for index in 0..11 {
            i64_gauge.record(
                1,
                [KeyValue::new(large_key.clone(), format!("{index}"))].as_ref(),
            );
        }

        for index in 0..11 {
            f64_gauge.record(
                1.0,
                [KeyValue::new(large_key.clone(), format!("{index}"))].as_ref(),
            );
        }
    }
}
