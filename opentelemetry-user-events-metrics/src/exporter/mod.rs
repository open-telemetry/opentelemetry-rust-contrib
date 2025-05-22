use opentelemetry::{otel_debug, otel_info};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::metrics::data;
use opentelemetry_sdk::metrics::exporter::PushMetricExporter;
use opentelemetry_sdk::metrics::{
    data::{
        ExponentialBucket, ExponentialHistogramDataPoint, Metric, ResourceMetrics, ScopeMetrics,
    },
    Temporality,
};

use crate::tracepoint;
use eventheader::_internal as ehi;
use prost::Message;
use std::fmt::{Debug, Formatter};
use std::pin::Pin;

const MAX_EVENT_SIZE: usize = 65360;

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

impl MetricsExporter {
    fn serialize_and_write(
        &self,
        resource_metric: &ResourceMetrics,
        metric_name: &str,
        metric_type: &str,
    ) -> OTelSdkResult {
        // Allocate a local buffer for each write operation
        // TODO: Investigate if this can be optimized to avoid reallocation or
        // allocate a fixed buffer size for all writes
        let mut byte_array = Vec::new();

        // Convert to proto message
        let proto_message: ExportMetricsServiceRequest = resource_metric.into();
        otel_debug!(name: "SerializeStart", 
            metric_name = metric_name,
            metric_type = metric_type);

        // Encode directly into the buffer
        match proto_message.encode(&mut byte_array) {
            Ok(_) => {
                otel_debug!(name: "SerializeSuccess", 
                    metric_name = metric_name,
                    metric_type = metric_type,
                    size = byte_array.len());
            }
            Err(err) => {
                otel_debug!(name: "SerializeFailed",
                    error = err.to_string(),
                    metric_name = metric_name,
                    metric_type = metric_type,
                    size = byte_array.len());
                return Err(OTelSdkError::InternalFailure(err.to_string()));
            }
        }

        // Check if the encoded message exceeds the 64 KB limit
        if byte_array.len() > MAX_EVENT_SIZE {
            otel_debug!(
                name: "MaxEventSizeExceeded",
                reason = format!("Encoded event size exceeds maximum allowed limit of {} bytes. Event will be dropped.", MAX_EVENT_SIZE),
                metric_name = metric_name,
                metric_type = metric_type,
                size = byte_array.len()
            );
            return Err(OTelSdkError::InternalFailure(
                "Event size exceeds maximum allowed limit".into(),
            ));
        }

        // Write to the tracepoint
        let result = tracepoint::write(&self.trace_point, &byte_array);
        if result > 0 {
            otel_debug!(name: "TracepointWrite", message = "Encoded data successfully written to tracepoint", size = byte_array.len(), metric_name = metric_name, metric_type = metric_type);
        }

        Ok(())
    }
}

impl PushMetricExporter for MetricsExporter {
    async fn export(&self, metrics: &mut ResourceMetrics) -> OTelSdkResult {
        otel_debug!(name: "ExportStart", message = "Starting metrics export");
        if !self.trace_point.enabled() {
            // TODO - This can flood the logs if the tracepoint is disabled for long periods of time
            otel_info!(name: "TracepointDisabled", message = "Tracepoint is disabled, skipping export");
            return Ok(());
        } else {
            let mut errors = Vec::new();

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
                                            start_time: histogram.start_time,
                                            time: histogram.time,
                                            data_points: vec![data_point.clone()],
                                        }),
                                    }],
                                }],
                            };
                            if let Err(e) = self.serialize_and_write(
                                &resource_metric,
                                &metric.name,
                                "Histogram<u64>",
                            ) {
                                errors.push(e.to_string());
                            }
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
                                            start_time: histogram.start_time,
                                            time: histogram.time,
                                            data_points: vec![data_point.clone()],
                                        }),
                                    }],
                                }],
                            };
                            if let Err(e) = self.serialize_and_write(
                                &resource_metric,
                                &metric.name,
                                "Histogram<f64>",
                            ) {
                                errors.push(e.to_string());
                            }
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
                                            start_time: gauge.start_time,
                                            time: gauge.time,
                                        }),
                                    }],
                                }],
                            };
                            if let Err(e) = self.serialize_and_write(
                                &resource_metric,
                                &metric.name,
                                "Gauge<u64>",
                            ) {
                                errors.push(e.to_string());
                            }
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
                                            start_time: gauge.start_time,
                                            time: gauge.time,
                                        }),
                                    }],
                                }],
                            };
                            if let Err(e) = self.serialize_and_write(
                                &resource_metric,
                                &metric.name,
                                "Gauge<i64>",
                            ) {
                                errors.push(e.to_string());
                            }
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
                                            start_time: gauge.start_time,
                                            time: gauge.time,
                                        }),
                                    }],
                                }],
                            };
                            if let Err(e) = self.serialize_and_write(
                                &resource_metric,
                                &metric.name,
                                "Gauge<f64>",
                            ) {
                                errors.push(e.to_string());
                            }
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
                                            start_time: sum.start_time,
                                            time: sum.time,
                                        }),
                                    }],
                                }],
                            };
                            if let Err(e) =
                                self.serialize_and_write(&resource_metric, &metric.name, "Sum<u64>")
                            {
                                errors.push(e.to_string());
                            }
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
                                            start_time: sum.start_time,
                                            time: sum.time,
                                        }),
                                    }],
                                }],
                            };
                            if let Err(e) =
                                self.serialize_and_write(&resource_metric, &metric.name, "Sum<i64>")
                            {
                                errors.push(e.to_string());
                            }
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
                                            start_time: sum.start_time,
                                            time: sum.time,
                                        }),
                                    }],
                                }],
                            };
                            if let Err(e) =
                                self.serialize_and_write(&resource_metric, &metric.name, "Sum<f64>")
                            {
                                errors.push(e.to_string());
                            }
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
                                            start_time: exp_hist.start_time,
                                            time: exp_hist.time,
                                            data_points: vec![ExponentialHistogramDataPoint {
                                                attributes: data_point.attributes.clone(),
                                                count: data_point.count,
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
                            if let Err(e) = self.serialize_and_write(
                                &resource_metric,
                                &metric.name,
                                "ExponentialHistogram<u64>",
                            ) {
                                errors.push(e.to_string());
                            }
                        }
                    } else if let Some(exp_hist) =
                        data.downcast_ref::<data::ExponentialHistogram<f64>>()
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
                                            start_time: exp_hist.start_time,
                                            time: exp_hist.time,
                                            data_points: vec![ExponentialHistogramDataPoint {
                                                attributes: data_point.attributes.clone(),
                                                count: data_point.count,
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
                            if let Err(e) = self.serialize_and_write(
                                &resource_metric,
                                &metric.name,
                                "ExponentialHistogram<f64>",
                            ) {
                                errors.push(e.to_string());
                            }
                        }
                    }
                }
            }

            // Return any errors if present
            if !errors.is_empty() {
                let error_message = format!(
                    "Export encountered {} errors: [{}]",
                    errors.len(),
                    errors.join("; ")
                );
                return Err(OTelSdkError::InternalFailure(error_message));
            }
        }
        Ok(())
    }

    fn temporality(&self) -> Temporality {
        Temporality::Delta
    }

    fn force_flush(&self) -> OTelSdkResult {
        Ok(()) // In this implementation, flush does nothing
    }

    fn shutdown(&self) -> OTelSdkResult {
        // TracepointState automatically deregisters when dropped
        // https://github.com/microsoft/LinuxTracepoints-Rust/blob/main/eventheader/src/native.rs#L618
        Ok(())
    }
}
