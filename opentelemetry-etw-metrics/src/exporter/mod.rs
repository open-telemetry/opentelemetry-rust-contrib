use opentelemetry::{
    global,
    metrics::{MetricsError, Result},
};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_sdk::metrics::{
    data::{
        self, ExponentialBucket, ExponentialHistogramDataPoint, Metric, ResourceMetrics,
        ScopeMetrics, Temporality,
    },
    exporter::PushMetricsExporter,
    reader::{AggregationSelector, DefaultAggregationSelector, TemporalitySelector},
    Aggregation, InstrumentKind,
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

impl TemporalitySelector for MetricsExporter {
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

impl AggregationSelector for MetricsExporter {
    fn aggregation(&self, kind: InstrumentKind) -> Aggregation {
        DefaultAggregationSelector::new().aggregation(kind)
    }
}

impl Debug for MetricsExporter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("ETW metrics exporter")
    }
}

#[async_trait]
impl PushMetricsExporter for MetricsExporter {
    async fn export(&self, metrics: &mut ResourceMetrics) -> Result<()> {
        for scope_metric in &metrics.scope_metrics {
            for metric in &scope_metric.metrics {
                let mut resource_metrics = Vec::new();

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
                        resource_metrics.push(resource_metric);
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
                        resource_metrics.push(resource_metric);
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
                        resource_metrics.push(resource_metric);
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
                        resource_metrics.push(resource_metric);
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
                        resource_metrics.push(resource_metric);
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
                        resource_metrics.push(resource_metric);
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
                        resource_metrics.push(resource_metric);
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
                        resource_metrics.push(resource_metric);
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
                        resource_metrics.push(resource_metric);
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
                        resource_metrics.push(resource_metric);
                    }
                } else {
                    global::handle_error(MetricsError::Other(format!(
                        "Unsupported aggregation type: {:?}",
                        data
                    )));
                }

                for resource_metric in resource_metrics {
                    let mut byte_array = Vec::new();
                    let proto_message: ExportMetricsServiceRequest = (&resource_metric).into();
                    proto_message
                        .encode(&mut byte_array)
                        .map_err(|err| MetricsError::Other(err.to_string()))?;

                    if (byte_array.len()) > etw::MAX_EVENT_SIZE {
                        global::handle_error(MetricsError::Other(format!(
                        "Exporting failed due to event size {} exceeding the maximum size of {} bytes",
                        byte_array.len(),
                        etw::MAX_EVENT_SIZE
                    )));
                    } else {
                        let result = etw::write(&byte_array);
                        // TODO: Better logging/internal metrics needed here for non-failure
                        // case Uncomment the line below to see the exported bytes until a
                        // better logging solution is implemented
                        // println!("Exported {} bytes to ETW", byte_array.len());
                        if result != 0 {
                            global::handle_error(MetricsError::Other(format!(
                                "Failed to write ETW event with error code: {}",
                                result
                            )));
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn force_flush(&self) -> Result<()> {
        Ok(())
    }

    fn shutdown(&self) -> Result<()> {
        etw::unregister();

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

    use crate::etw;

    #[tokio::test(flavor = "multi_thread")]
    async fn fail_to_export_too_many_metrics() {
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
        let c = meter
            .f64_counter("TestCounter")
            .with_description("test_description")
            .with_unit("test_unit")
            .init();

        // Create a key that is 1/10th the size of the MAX_EVENT_SIZE
        let key_size = etw::MAX_EVENT_SIZE / 10;
        let large_key = "a".repeat(key_size);

        for index in 0..11 {
            c.add(
                1.0,
                [KeyValue::new(large_key.clone(), format!("{index}"))].as_ref(),
            );
        }
    }
}
