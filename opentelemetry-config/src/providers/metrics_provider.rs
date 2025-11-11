//! Provider for Metrics telemetry
//!
//! This module provides functionality to configure Metrics telemetry
//! in OpenTelemetry SDKs using declarative YAML configurations.

pub mod reader_provider;

use opentelemetry_sdk::metrics::MeterProviderBuilder;

use crate::{MetricsProvidersRegistry, ProviderError};

use crate::providers::metrics_provider::reader_provider::ReaderProvider;

/// Provider for Metrics telemetry
pub struct MetricsProvider {
    reader_provider: ReaderProvider,
}

impl MetricsProvider {
    pub fn new() -> Self {
        MetricsProvider {
            reader_provider: ReaderProvider::new(),
        }
    }

    /// Provisions the Metrics provider based on the provided configuration
    pub fn provide(
        &self,
        metrics_registry: &MetricsProvidersRegistry,
        mut meter_provider_builder: MeterProviderBuilder,
        config: &crate::model::metrics::Metrics,
    ) -> Result<MeterProviderBuilder, ProviderError> {
        for reader in &config.readers {
            meter_provider_builder =
                self.reader_provider
                    .provide(metrics_registry, meter_provider_builder, reader)?;
        }

        Ok(meter_provider_builder)
    }
}

impl Default for MetricsProvider {
    fn default() -> Self {
        Self::new()
    }
}
