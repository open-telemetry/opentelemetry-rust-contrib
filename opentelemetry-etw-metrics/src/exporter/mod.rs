use opentelemetry::metrics::{MetricsError, Result};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_sdk::metrics::{
    data::{ResourceMetrics, Temporality},
    exporter::PushMetricsExporter,
    reader::{AggregationSelector, DefaultAggregationSelector, TemporalitySelector},
    Aggregation, InstrumentKind,
};

use async_trait::async_trait;
use prost::Message;
use tracelogging;

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

        let mut byte_array = Vec::new();
        proto_message
            .encode(&mut byte_array)
            .map_err(|err| MetricsError::Other(err.to_string()))?;

        let result = etw::write(&byte_array);

        match result {
            0 => println!("Successfully wrote ETW event"),
            error_code => eprintln!("Failed to write ETW event with error code: {}", error_code),
        }

        Ok(())
    }

    async fn force_flush(&self) -> Result<()> {
        Ok(())
    }

    /// TODO: What do we do if no one calls shutdown? Can we use drop somehow?
    fn shutdown(&self) -> Result<()> {
        etw::unregister();

        Ok(())
    }
}
