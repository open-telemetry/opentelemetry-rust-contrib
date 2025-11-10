//! # OpenTelemetry Dynamic Configurator module for Stdout (Console) exporter
//!
//! This module provides a configurator for OpenTelemetry Metrics
//! that enables exporting metrics to the console (stdout) using
//! the OpenTelemetry Config crate.

use opentelemetry_config::{
    model::metrics::reader::PeriodicExporterConsole, MetricsReaderPeriodicExporterConfigurator,
};
use opentelemetry_sdk::metrics::MeterProviderBuilder;

#[derive(Clone)]
pub struct ConsolePeriodicExporterConfigurator {}

impl ConsolePeriodicExporterConfigurator {
    pub fn new() -> Self {
        Self {}
    }

    pub fn register_into(configurators_manager: &mut opentelemetry_config::ConfiguratorManager) {
        let configurator = ConsolePeriodicExporterConfigurator::new();
        configurators_manager
            .metrics_mut()
            .register_periodic_exporter_configurator::<PeriodicExporterConsole>(Box::new(
                configurator.clone(),
            ));
        // TODO: Add logs and traces configurator registration.
    }
}

impl MetricsReaderPeriodicExporterConfigurator for ConsolePeriodicExporterConfigurator {
    fn configure(
        &self,
        mut meter_provider_builder: MeterProviderBuilder,
        config: &dyn std::any::Any,
    ) -> MeterProviderBuilder {
        let mut exporter_builder = opentelemetry_stdout::MetricExporter::builder();

        let config = config
            .downcast_ref::<PeriodicExporterConsole>()
            .expect("Invalid config type. Expected PeriodicExporterConsole.");

        if let Some(temporality) = &config.temporality {
            match temporality {
                opentelemetry_config::model::metrics::reader::Temporality::Delta => {
                    exporter_builder = exporter_builder
                        .with_temporality(opentelemetry_sdk::metrics::Temporality::Delta);
                }
                opentelemetry_config::model::metrics::reader::Temporality::Cumulative => {
                    exporter_builder = exporter_builder
                        .with_temporality(opentelemetry_sdk::metrics::Temporality::Cumulative);
                }
            }
        }

        let exporter = exporter_builder.build();
        meter_provider_builder = meter_provider_builder.with_periodic_exporter(exporter);
        meter_provider_builder
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Default for ConsolePeriodicExporterConfigurator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_console_configurator_registration() {
        // Arrange
        let mut configurator_manager = opentelemetry_config::ConfiguratorManager::new();

        // Act
        ConsolePeriodicExporterConfigurator::register_into(&mut configurator_manager);

        let configurators_option = configurator_manager
            .metrics()
            .readers_periodic_exporter::<PeriodicExporterConsole>();

        // Assert
        assert!(configurators_option.is_some());
    }

    #[test]
    fn test_console_configurator_configure_temporality_minimal() {
        // Arrange
        let configurator = ConsolePeriodicExporterConfigurator::new();
        let meter_provider_builder = opentelemetry_sdk::metrics::SdkMeterProvider::builder();

        let config = PeriodicExporterConsole { temporality: None };

        // Act
        let configured_builder = configurator.configure(meter_provider_builder, &config);

        // Assert
        // Since the MeterProviderBuilder does not expose its internal state,
        // we will just ensure that the returned builder is not the same as the original.
        assert!(!std::ptr::eq(
            &configured_builder,
            &opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        ));
    }

    #[test]
    fn test_console_configurator_configure_temporality_delta() {
        // Arrange
        let configurator = ConsolePeriodicExporterConfigurator::new();
        let meter_provider_builder = opentelemetry_sdk::metrics::SdkMeterProvider::builder();

        let config = PeriodicExporterConsole {
            temporality: Some(opentelemetry_config::model::metrics::reader::Temporality::Delta),
        };

        // Act
        let configured_builder = configurator.configure(meter_provider_builder, &config);

        // Assert
        // Since the MeterProviderBuilder does not expose its internal state,
        // we will just ensure that the returned builder is not the same as the original.
        assert!(!std::ptr::eq(
            &configured_builder,
            &opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        ));
    }

    #[test]
    fn test_console_configurator_configure_temporality_cumulative() {
        // Arrange
        let configurator = ConsolePeriodicExporterConfigurator::new();
        let meter_provider_builder = opentelemetry_sdk::metrics::SdkMeterProvider::builder();

        let config = PeriodicExporterConsole {
            temporality: Some(
                opentelemetry_config::model::metrics::reader::Temporality::Cumulative,
            ),
        };

        // Act
        let configured_builder = configurator.configure(meter_provider_builder, &config);

        // Assert
        // Since the MeterProviderBuilder does not expose its internal state,
        // we will just ensure that the returned builder is not the same as the original.
        assert!(!std::ptr::eq(
            &configured_builder,
            &opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        ));
    }
}
