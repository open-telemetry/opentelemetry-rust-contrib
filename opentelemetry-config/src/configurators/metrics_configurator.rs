//! Configurator for Metrics telemetry
//!
//! This module provides functionality to configure Metrics telemetry
//! in OpenTelemetry SDKs using declarative YAML configurations.

pub mod reader_configurator;

use opentelemetry_sdk::metrics::MeterProviderBuilder;

use crate::{ConfiguratorError, MetricsConfiguratorManager};

use crate::configurators::metrics_configurator::reader_configurator::ReaderConfigurator;

/// Configurator for Metrics telemetry
pub struct MetricsConfigurator {
    reader_configurator: ReaderConfigurator,
}

impl MetricsConfigurator {
    pub fn new() -> Self {
        MetricsConfigurator {
            reader_configurator: ReaderConfigurator::new(),
        }
    }

    /// Configures the Metrics provider based on the provided configuration
    pub fn configure(
        &self,
        metrics_configurator_manager: &MetricsConfiguratorManager,
        mut meter_provider_builder: MeterProviderBuilder,
        config: &crate::model::metrics::Metrics,
    ) -> Result<MeterProviderBuilder, ConfiguratorError> {
        for reader in &config.readers {
            meter_provider_builder = self.reader_configurator.configure(
                metrics_configurator_manager,
                meter_provider_builder,
                reader,
            )?;
        }

        Ok(meter_provider_builder)
    }
}

impl Default for MetricsConfigurator {
    fn default() -> Self {
        Self::new()
    }
}
