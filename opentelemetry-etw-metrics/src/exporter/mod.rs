use opentelemetry::{
    global,
    metrics::{MetricsError, Result},
};
use opentelemetry_proto::tonic::{
    collector::metrics::v1::ExportMetricsServiceRequest,
    metrics::v1::{
        metric::Data as TonicMetricData, number_data_point::Value as TonicDataPointValue,
        Metric as TonicMetric, ResourceMetrics as TonicMetrics,
        ResourceMetrics as TonicResourceMetrics, ScopeMetrics as TonicScopeMetrics,
        Sum as TonicSum,
    },
};
use opentelemetry_sdk::metrics::{
    data::{self, Metric, ResourceMetrics, Temporality},
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
        let proto_message: ExportMetricsServiceRequest = (&*metrics).into();
        // ExportMetricsServiceRequest -> ResourceMetrics -> ScopeMetrics -> Metric -> Aggregation -> Gauge

        for scope_metric in &metrics.scope_metrics {
            for metric in &scope_metric.metrics {
                let data = &metric.data.as_any();
                if let Some(hist) = data.downcast_ref::<data::Histogram<u64>>() {
                    println!("u64 Histogram");
                } else if let Some(hist) = data.downcast_ref::<data::Histogram<f64>>() {
                    println!("f64 Histogram");
                } else if let Some(_hist) = data.downcast_ref::<data::ExponentialHistogram<u64>>() {
                    println!("Exponential Histogram");
                } else if let Some(_hist) = data.downcast_ref::<data::ExponentialHistogram<f64>>() {
                    println!("Exponential Histogram");
                } else if let Some(sum) = data.downcast_ref::<data::Sum<u64>>() {
                    println!("u64 Sum");
                } else if let Some(sum) = data.downcast_ref::<data::Sum<i64>>() {
                    println!("i64 Sum");
                } else if let Some(sum) = data.downcast_ref::<data::Sum<f64>>() {
                    println!("f64 Sum");

                    let tonic_sum: TonicSum = (&*sum).into();
                    for data_point in tonic_sum.data_points {
                        let export_metric_service_request = ExportMetricsServiceRequest {
                            resource_metrics: vec![TonicMetrics {
                                resource: Some((&metrics.resource).into()),
                                scope_metrics: vec![TonicScopeMetrics {
                                    scope: Some((scope_metric.scope.clone(), None).into()),
                                    metrics: vec![TonicMetric {
                                        name: metric.name.to_string(),
                                        description: metric.description.to_string(),
                                        unit: metric.unit.to_string(),
                                        metadata: vec![],
                                        data: Some(TonicMetricData::Sum(TonicSum {
                                            aggregation_temporality: tonic_sum
                                                .aggregation_temporality,
                                            data_points: vec![data_point],
                                            is_monotonic: tonic_sum.is_monotonic,
                                        })),
                                    }],
                                    schema_url: scope_metric
                                        .scope
                                        .schema_url
                                        .as_ref()
                                        .map(ToString::to_string)
                                        .unwrap_or_default(),
                                }],
                                schema_url: metrics
                                    .resource
                                    .schema_url()
                                    .map(Into::into)
                                    .unwrap_or_default(),
                            }],
                        };

                        let mut byte_array = Vec::new();
                        export_metric_service_request
                            .encode(&mut byte_array)
                            .map_err(|err| MetricsError::Other(err.to_string()))?;

                        let result = etw::write(&byte_array);
                        if result != 0 {
                            global::handle_error(MetricsError::Other(format!(
                                "Failed to write ETW event with error code: {}",
                                result
                            )));
                        }
                    }
                } else if let Some(gauge) = data.downcast_ref::<data::Gauge<u64>>() {
                    println!("u64 Gauge");
                } else if let Some(gauge) = data.downcast_ref::<data::Gauge<i64>>() {
                    println!("i64 Gauge");
                } else if let Some(gauge) = data.downcast_ref::<data::Gauge<f64>>() {
                    println!("f64 Gauge");
                } else {
                    println!("Unsupported data type");
                }
            }
        }

        // let mut byte_array = Vec::new();
        // proto_message
        //     .encode(&mut byte_array)
        //     .map_err(|err| MetricsError::Other(err.to_string()))?;

        // if (byte_array.len()) > etw::MAX_EVENT_SIZE {
        //     global::handle_error(MetricsError::Other(format!(
        //         "Exporting failed due to event size {} exceeding the maximum size of {} bytes",
        //         byte_array.len(),
        //         etw::MAX_EVENT_SIZE
        //     )));
        // } else {
        //     let result = etw::write(&byte_array);
        //     // TODO: Better logging/internal metrics needed here for non-failure
        //     // case Uncomment the line below to see the exported bytes until a
        //     // better logging solution is implemented
        //     // println!("Exported {} bytes to ETW", byte_array.len());
        //     if result != 0 {
        //         global::handle_error(MetricsError::Other(format!(
        //             "Failed to write ETW event with error code: {}",
        //             result
        //         )));
        //     }
        // }
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

        for index in 0..etw::MAX_EVENT_SIZE {
            c.add(1.0, [KeyValue::new("index", format!("{index}"))].as_ref());
        }
    }
}
