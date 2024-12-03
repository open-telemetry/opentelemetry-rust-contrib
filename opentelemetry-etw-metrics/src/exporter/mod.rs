use opentelemetry::otel_warn;

use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_sdk::metrics::{
    data::{
        self, ExponentialBucket, ExponentialHistogramDataPoint, Metric, ResourceMetrics,
        ScopeMetrics,
    },
    exporter::PushMetricExporter,
    MetricError, MetricResult, Temporality,
};
use prost::Message;

use async_trait::async_trait;

use std::fmt::{Debug, Formatter};

use crate::etw;

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

fn emit_metric(resource_metric: &ResourceMetrics, buffer: &mut Vec<u8>) -> MetricResult<()> {
    // Zero the buffer to ensure no data is left over from previous writes
    buffer.clear();

    let proto_message: ExportMetricsServiceRequest = (&*resource_metric).into();
    proto_message
        .encode(buffer)
        .map_err(|err| MetricError::Other(err.to_string()))?;

    if (proto_message.encoded_len()) > etw::MAX_EVENT_SIZE {
        otel_warn!(name: "MetricExportFailedDueToMaxSizeLimit", size = proto_message.encoded_len(), max_size = etw::MAX_EVENT_SIZE);
    } else {
        let result = etw::write(&buffer);
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

#[async_trait]
impl PushMetricExporter for MetricsExporter {
    async fn export(&self, metrics: &mut ResourceMetrics) -> MetricResult<()> {
        let mut encoding_buffer: Vec<u8> = Vec::with_capacity(1024);

        for scope_metric in &metrics.scope_metrics {
            for metric in &scope_metric.metrics {
                let data = &metric.data.as_any();
                if let Some(hist) = data.downcast_ref::<data::Histogram<u64>>() {
                    for data_point in &hist.data_points {
                        let resource_metric = ResourceMetrics {
                            resource: metrics.resource.clone(),
                            scope_metrics: vec![ScopeMetrics {
                                scope: scope_metric.scope.clone(),
                                metrics: vec![Metric {
                                    name: metric.name.clone(),
                                    description: metric.description.clone(),
                                    unit: metric.unit.clone(),
                                    data: Box::new(data::Histogram {
                                        temporality: hist.temporality,
                                        data_points: vec![data_point.clone()],
                                    }),
                                }],
                            }],
                        };
                        emit_metric(&resource_metric, &mut encoding_buffer)?;
                    }
                } else if let Some(hist) = data.downcast_ref::<data::Histogram<f64>>() {
                    for data_point in &hist.data_points {
                        let resource_metric = ResourceMetrics {
                            resource: metrics.resource.clone(),
                            scope_metrics: vec![ScopeMetrics {
                                scope: scope_metric.scope.clone(),
                                metrics: vec![Metric {
                                    name: metric.name.clone(),
                                    description: metric.description.clone(),
                                    unit: metric.unit.clone(),
                                    data: Box::new(data::Histogram {
                                        temporality: hist.temporality,
                                        data_points: vec![data_point.clone()],
                                    }),
                                }],
                            }],
                        };
                        emit_metric(&resource_metric, &mut encoding_buffer)?;
                    }
                } else if let Some(hist) = data.downcast_ref::<data::ExponentialHistogram<u64>>() {
                    for data_point in &hist.data_points {
                        let resource_metric = ResourceMetrics {
                            resource: metrics.resource.clone(),
                            scope_metrics: vec![ScopeMetrics {
                                scope: scope_metric.scope.clone(),
                                metrics: vec![Metric {
                                    name: metric.name.clone(),
                                    description: metric.description.clone(),
                                    unit: metric.unit.clone(),
                                    data: Box::new(data::ExponentialHistogram {
                                        temporality: hist.temporality,
                                        data_points: vec![ExponentialHistogramDataPoint {
                                            attributes: data_point.attributes.clone(),
                                            count: data_point.count,
                                            start_time: data_point.start_time,
                                            time: data_point.time,
                                            min: data_point.min,
                                            max: data_point.max,
                                            sum: data_point.sum,
                                            scale: data_point.scale,
                                            zero_count: data_point.zero_count,
                                            zero_threshold: data_point.zero_threshold,
                                            positive_bucket: ExponentialBucket {
                                                offset: data_point.positive_bucket.offset,
                                                counts: data_point.positive_bucket.counts.clone(),
                                            },
                                            negative_bucket: ExponentialBucket {
                                                offset: data_point.negative_bucket.offset,
                                                counts: data_point.negative_bucket.counts.clone(),
                                            },
                                            exemplars: data_point.exemplars.clone(),
                                        }],
                                    }),
                                }],
                            }],
                        };
                        emit_metric(&resource_metric, &mut encoding_buffer)?;
                    }
                } else if let Some(hist) = data.downcast_ref::<data::ExponentialHistogram<f64>>() {
                    for data_point in &hist.data_points {
                        let resource_metric = ResourceMetrics {
                            resource: metrics.resource.clone(),
                            scope_metrics: vec![ScopeMetrics {
                                scope: scope_metric.scope.clone(),
                                metrics: vec![Metric {
                                    name: metric.name.clone(),
                                    description: metric.description.clone(),
                                    unit: metric.unit.clone(),
                                    data: Box::new(data::ExponentialHistogram {
                                        temporality: hist.temporality,
                                        data_points: vec![ExponentialHistogramDataPoint {
                                            attributes: data_point.attributes.clone(),
                                            count: data_point.count,
                                            start_time: data_point.start_time,
                                            time: data_point.time,
                                            min: data_point.min,
                                            max: data_point.max,
                                            sum: data_point.sum,
                                            scale: data_point.scale,
                                            zero_count: data_point.zero_count,
                                            zero_threshold: data_point.zero_threshold,
                                            positive_bucket: ExponentialBucket {
                                                offset: data_point.positive_bucket.offset,
                                                counts: data_point.positive_bucket.counts.clone(),
                                            },
                                            negative_bucket: ExponentialBucket {
                                                offset: data_point.negative_bucket.offset,
                                                counts: data_point.negative_bucket.counts.clone(),
                                            },
                                            exemplars: data_point.exemplars.clone(),
                                        }],
                                    }),
                                }],
                            }],
                        };
                        emit_metric(&resource_metric, &mut encoding_buffer)?;
                    }
                } else if let Some(sum) = data.downcast_ref::<data::Sum<u64>>() {
                    for data_point in &sum.data_points {
                        let resource_metric = ResourceMetrics {
                            resource: metrics.resource.clone(),
                            scope_metrics: vec![ScopeMetrics {
                                scope: scope_metric.scope.clone(),
                                metrics: vec![Metric {
                                    name: metric.name.clone(),
                                    description: metric.description.clone(),
                                    unit: metric.unit.clone(),
                                    data: Box::new(data::Sum {
                                        temporality: sum.temporality,
                                        data_points: vec![data_point.clone()],
                                        is_monotonic: sum.is_monotonic,
                                    }),
                                }],
                            }],
                        };
                        emit_metric(&resource_metric, &mut encoding_buffer)?;
                    }
                } else if let Some(sum) = data.downcast_ref::<data::Sum<i64>>() {
                    for data_point in &sum.data_points {
                        let resource_metric = ResourceMetrics {
                            resource: metrics.resource.clone(),
                            scope_metrics: vec![ScopeMetrics {
                                scope: scope_metric.scope.clone(),
                                metrics: vec![Metric {
                                    name: metric.name.clone(),
                                    description: metric.description.clone(),
                                    unit: metric.unit.clone(),
                                    data: Box::new(data::Sum {
                                        temporality: sum.temporality,
                                        data_points: vec![data_point.clone()],
                                        is_monotonic: sum.is_monotonic,
                                    }),
                                }],
                            }],
                        };
                        emit_metric(&resource_metric, &mut encoding_buffer)?;
                    }
                } else if let Some(sum) = data.downcast_ref::<data::Sum<f64>>() {
                    for data_point in &sum.data_points {
                        let resource_metric = ResourceMetrics {
                            resource: metrics.resource.clone(),
                            scope_metrics: vec![ScopeMetrics {
                                scope: scope_metric.scope.clone(),
                                metrics: vec![Metric {
                                    name: metric.name.clone(),
                                    description: metric.description.clone(),
                                    unit: metric.unit.clone(),
                                    data: Box::new(data::Sum {
                                        temporality: sum.temporality,
                                        data_points: vec![data_point.clone()],
                                        is_monotonic: sum.is_monotonic,
                                    }),
                                }],
                            }],
                        };
                        emit_metric(&resource_metric, &mut encoding_buffer)?;
                    }
                } else if let Some(gauge) = data.downcast_ref::<data::Gauge<u64>>() {
                    for data_point in &gauge.data_points {
                        let resource_metric = ResourceMetrics {
                            resource: metrics.resource.clone(),
                            scope_metrics: vec![ScopeMetrics {
                                scope: scope_metric.scope.clone(),
                                metrics: vec![Metric {
                                    name: metric.name.clone(),
                                    description: metric.description.clone(),
                                    unit: metric.unit.clone(),
                                    data: Box::new(data::Gauge {
                                        data_points: vec![data_point.clone()],
                                    }),
                                }],
                            }],
                        };
                        emit_metric(&resource_metric, &mut encoding_buffer)?;
                    }
                } else if let Some(gauge) = data.downcast_ref::<data::Gauge<i64>>() {
                    for data_point in &gauge.data_points {
                        let resource_metric = ResourceMetrics {
                            resource: metrics.resource.clone(),
                            scope_metrics: vec![ScopeMetrics {
                                scope: scope_metric.scope.clone(),
                                metrics: vec![Metric {
                                    name: metric.name.clone(),
                                    description: metric.description.clone(),
                                    unit: metric.unit.clone(),
                                    data: Box::new(data::Gauge {
                                        data_points: vec![data_point.clone()],
                                    }),
                                }],
                            }],
                        };
                        emit_metric(&resource_metric, &mut encoding_buffer)?;
                    }
                } else if let Some(gauge) = data.downcast_ref::<data::Gauge<f64>>() {
                    for data_point in &gauge.data_points {
                        let resource_metric = ResourceMetrics {
                            resource: metrics.resource.clone(),
                            scope_metrics: vec![ScopeMetrics {
                                scope: scope_metric.scope.clone(),
                                metrics: vec![Metric {
                                    name: metric.name.clone(),
                                    description: metric.description.clone(),
                                    unit: metric.unit.clone(),
                                    data: Box::new(data::Gauge {
                                        data_points: vec![data_point.clone()],
                                    }),
                                }],
                            }],
                        };
                        emit_metric(&resource_metric, &mut encoding_buffer)?;
                    }
                } else {
                    otel_warn!(name: "MetricExportFailedDueToUnsupportedMetricType", metric_type = format!("{:?}", data));
                }
            }
        }

        Ok(())
    }

    async fn force_flush(&self) -> MetricResult<()> {
        Ok(())
    }

    fn shutdown(&self) -> MetricResult<()> {
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
        runtime, Resource,
    };

    use crate::etw;

    #[tokio::test(flavor = "multi_thread")]
    async fn emit_metrics_that_combined_exceed_etw_max_event_size() {
        let exporter = super::MetricsExporter::new();
        let reader = PeriodicReader::builder(exporter, runtime::Tokio).build();
        let meter_provider = SdkMeterProvider::builder()
            .with_resource(Resource::new(vec![KeyValue::new(
                "service.name",
                "service-name",
            )]))
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
