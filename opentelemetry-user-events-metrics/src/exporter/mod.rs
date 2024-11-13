use async_trait::async_trait;
use opentelemetry::metrics::{MetricsError, Result};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_sdk::metrics::data;
use opentelemetry_sdk::metrics::{
    data::{
        ExponentialBucket, ExponentialHistogramDataPoint, Metric, ResourceMetrics, ScopeMetrics,
        Temporality,
    },
    exporter::PushMetricsExporter,
    reader::TemporalitySelector,
    InstrumentKind,
};

use crate::tracepoint;
use eventheader::_internal as ehi;
use prost::Message;
use std::fmt::{Debug, Formatter};
use std::pin::Pin;

const MAX_EVENT_SIZE: usize = 64 * 1024; // 64 KB

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

impl TemporalitySelector for MetricsExporter {
    // This is matching OTLP exporters delta.
    fn temporality(&self, kind: InstrumentKind) -> Temporality {
        match kind {
            InstrumentKind::Counter
            | InstrumentKind::ObservableCounter
            | InstrumentKind::ObservableGauge
            | InstrumentKind::Histogram
            | InstrumentKind::Gauge => Temporality::Delta,
            InstrumentKind::UpDownCounter | InstrumentKind::ObservableUpDownCounter => {
                Temporality::Cumulative
            }
        }
    }
}

impl Debug for MetricsExporter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("user_events metrics exporter")
    }
}

impl MetricsExporter {
    async fn serialize_and_write(&self, resource_metric: &ResourceMetrics) -> Result<()> {
        // Allocate a local buffer for each write operation
        let mut byte_array = Vec::new();

        // Convert to proto message
        let proto_message: ExportMetricsServiceRequest = resource_metric.into();

        // Encode directly into the buffer
        proto_message
            .encode(&mut byte_array)
            .map_err(|err| MetricsError::Other(err.to_string()))?;

        // Check if the encoded message exceeds the 64 KB limit
        if byte_array.len() > MAX_EVENT_SIZE {
            return Err(MetricsError::Other(
                "Event size exceeds maximum allowed limit".into(),
            ));
        }

        // Write to the tracepoint
        tracepoint::write(&self.trace_point, &byte_array);
        Ok(())
    }
}

#[async_trait]
impl PushMetricsExporter for MetricsExporter {
    async fn export(&self, metrics: &mut ResourceMetrics) -> Result<()> {
        if self.trace_point.enabled() {
            let mut resource_metrics_list = Vec::new();

            for scope_metric in &metrics.scope_metrics {
                for metric in &scope_metric.metrics {
                    let data = &metric.data.as_any();

                    if let Some(histogram) = data.downcast_ref::<data::Histogram<u64>>() {
                        for data_point in &histogram.data_points {
                            let resource_metric = ResourceMetrics {
                                resource: metrics.resource.clone(),
                                scope_metrics: vec![ScopeMetrics {
                                    scope: scope_metric.scope.clone(),
                                    metrics: vec![Metric {
                                        name: metric.name.clone(),
                                        description: metric.description.clone(),
                                        unit: metric.unit.clone(),
                                        data: Box::new(data::Histogram {
                                            temporality: histogram.temporality,
                                            data_points: vec![data_point.clone()],
                                        }),
                                    }],
                                }],
                            };
                            resource_metrics_list.push(resource_metric);
                        }
                    } else if let Some(histogram) = data.downcast_ref::<data::Histogram<f64>>() {
                        for data_point in &histogram.data_points {
                            let resource_metric = ResourceMetrics {
                                resource: metrics.resource.clone(),
                                scope_metrics: vec![ScopeMetrics {
                                    scope: scope_metric.scope.clone(),
                                    metrics: vec![Metric {
                                        name: metric.name.clone(),
                                        description: metric.description.clone(),
                                        unit: metric.unit.clone(),
                                        data: Box::new(data::Histogram {
                                            temporality: histogram.temporality,
                                            data_points: vec![data_point.clone()],
                                        }),
                                    }],
                                }],
                            };
                            resource_metrics_list.push(resource_metric);
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
                            resource_metrics_list.push(resource_metric);
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
                            resource_metrics_list.push(resource_metric);
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
                            resource_metrics_list.push(resource_metric);
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
                            resource_metrics_list.push(resource_metric);
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
                            resource_metrics_list.push(resource_metric);
                        }
                    } else if let Some(exp_hist) =
                        data.downcast_ref::<data::ExponentialHistogram<u64>>()
                    {
                        for data_point in &exp_hist.data_points {
                            let resource_metric = ResourceMetrics {
                                resource: metrics.resource.clone(),
                                scope_metrics: vec![ScopeMetrics {
                                    scope: scope_metric.scope.clone(),
                                    metrics: vec![Metric {
                                        name: metric.name.clone(),
                                        description: metric.description.clone(),
                                        unit: metric.unit.clone(),
                                        data: Box::new(data::ExponentialHistogram {
                                            temporality: exp_hist.temporality,
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
                                                    counts: data_point
                                                        .positive_bucket
                                                        .counts
                                                        .clone(),
                                                },
                                                negative_bucket: ExponentialBucket {
                                                    offset: data_point.negative_bucket.offset,
                                                    counts: data_point
                                                        .negative_bucket
                                                        .counts
                                                        .clone(),
                                                },
                                                exemplars: data_point.exemplars.clone(),
                                            }],
                                        }),
                                    }],
                                }],
                            };
                            resource_metrics_list.push(resource_metric);
                        }
                    }
                }
            }

            // Asynchronously serialize and write each ResourceMetrics to tracepoint
            for resource_metric in resource_metrics_list {
                self.serialize_and_write(&resource_metric).await?;
            }
        }
        Ok(())
    }

    async fn force_flush(&self) -> Result<()> {
        Ok(()) // In this implementation, flush does nothing
    }

    fn shutdown(&self) -> Result<()> {
        // TracepointState automatically unregisters when dropped
        // https://github.com/microsoft/LinuxTracepoints-Rust/blob/main/eventheader/src/native.rs#L618
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use opentelemetry::{metrics::MeterProvider as _, KeyValue};
    use opentelemetry_sdk::{
        metrics::{PeriodicReader, SdkMeterProvider},
        runtime, Resource,
    };

    use crate::exporter::MAX_EVENT_SIZE;
    use crate::MetricsExporter; // Ensure this references the correct exporter module

    #[tokio::test(flavor = "multi_thread")]
    async fn emit_metrics_that_combined_exceed_max_event_size() {
        let exporter = MetricsExporter::new();
        let reader = PeriodicReader::builder(exporter, runtime::Tokio).build();
        let meter_provider = SdkMeterProvider::builder()
            .with_resource(Resource::new(vec![KeyValue::new(
                "service.name",
                "service-name",
            )]))
            .with_reader(reader)
            .build();

        let meter = meter_provider.meter("user-event-test");

        // Initialize metric types
        let u64_histogram = meter
            .u64_histogram("Testu64Histogram")
            .with_description("u64_histogram_test_description")
            .with_unit("u64_histogram_test_unit")
            .init();

        let u64_counter = meter
            .u64_counter("Testu64Counter")
            .with_description("u64_counter_test_description")
            .with_unit("u64_counter_test_units")
            .init();

        let u64_gauge = meter
            .u64_gauge("Testu64Gauge")
            .with_description("u64_gauge_test_description")
            .with_unit("u64_gauge_test_unit")
            .init();

        // Generate a large key to fill the buffer
        let key_size = MAX_EVENT_SIZE / 10;
        let large_key = "a".repeat(key_size);

        // Record data with large attributes to ensure size limits are exceeded
        for index in 0..11 {
            u64_histogram.record(
                1,
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
            u64_gauge.record(
                1,
                [KeyValue::new(large_key.clone(), format!("{index}"))].as_ref(),
            );
        }

        // The output will be verified through logs or the handling of oversized messages in the code.
        // You may also consider adding assertions for logging or verifying how oversize events are discarded.
    }
}
