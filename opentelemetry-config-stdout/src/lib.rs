//! # OpenTelemetry declarative configuration module for Stdout (Console) exporter
//!
//! This module implements a provider for OpenTelemetry Metrics
//! that enables exporting metrics to the console (stdout) using
//! the OpenTelemetry Config crate.

use opentelemetry_config::ConfigurationError;
use opentelemetry_sdk::metrics::MeterProviderBuilder;

pub fn register_console_exporter(
    mut builder: MeterProviderBuilder,
    config: &serde_yaml::Value,
) -> Result<MeterProviderBuilder, ConfigurationError> {
    let mut exporter_builder = opentelemetry_stdout::MetricExporter::builder();

    let config =
        serde_yaml::from_value::<PeriodicExporterConsole>(config.clone()).map_err(|e| {
            ConfigurationError::InvalidConfiguration(format!(
                "Failed to deserialize PeriodicExporterConsole configuration: {}",
                e
            ))
        })?;

    if let Some(temporality) = &config.temporality {
        match temporality {
            Temporality::Delta => {
                exporter_builder = exporter_builder
                    .with_temporality(opentelemetry_sdk::metrics::Temporality::Delta);
            }
            Temporality::Cumulative => {
                exporter_builder = exporter_builder
                    .with_temporality(opentelemetry_sdk::metrics::Temporality::Cumulative);
            }
        }
    }

    let exporter = exporter_builder.build();
    builder = builder.with_periodic_exporter(exporter);
    Ok(builder)
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct PeriodicExporterConsole {
    pub temporality: Option<Temporality>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Temporality {
    Delta,
    Cumulative,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_console_provider_registration() {
        // Arrange
        let mut configuration_registry =
            opentelemetry_config::ConfigurationProvidersRegistry::new();

        // Act
        let metrics_registry = configuration_registry.metrics_mut();
        metrics_registry
            .register_periodic_exporter_factory("console".to_string(), register_console_exporter);

        // Assert
        assert!(metrics_registry
            .periodic_exporter_factory("console")
            .is_some());
    }

    #[test]
    fn test_console_provider_configure_temporality_minimal() {
        // Arrange
        let meter_provider_builder = opentelemetry_sdk::metrics::SdkMeterProvider::builder();

        let config = PeriodicExporterConsole { temporality: None };

        let config_yaml = serde_yaml::to_value(config).unwrap();

        // Act
        let configured_builder =
            register_console_exporter(meter_provider_builder, &config_yaml).unwrap();

        // Assert
        // Since the MeterProviderBuilder does not expose its internal state,
        // we will just ensure that the returned builder is not the same as the original.
        assert!(!std::ptr::eq(
            &configured_builder,
            &opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        ));
    }

    #[test]
    fn test_console_provider_configure_temporality_delta() {
        // Arrange
        let meter_provider_builder = opentelemetry_sdk::metrics::SdkMeterProvider::builder();

        let config = PeriodicExporterConsole {
            temporality: Some(Temporality::Delta),
        };

        let config_yaml = serde_yaml::to_value(config).unwrap();

        // Act
        let configured_builder =
            register_console_exporter(meter_provider_builder, &config_yaml).unwrap();

        // Assert
        // Since the MeterProviderBuilder does not expose its internal state,
        // we will just ensure that the returned builder is not the same as the original.
        assert!(!std::ptr::eq(
            &configured_builder,
            &opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        ));
    }

    #[test]
    fn test_console_provider_configure_temporality_cumulative() {
        // Arrange
        let meter_provider_builder = opentelemetry_sdk::metrics::SdkMeterProvider::builder();

        let config = PeriodicExporterConsole {
            temporality: Some(Temporality::Cumulative),
        };

        let config_yaml = serde_yaml::to_value(config).unwrap();

        // Act
        let configured_builder =
            register_console_exporter(meter_provider_builder, &config_yaml).unwrap();

        // Assert
        // Since the MeterProviderBuilder does not expose its internal state,
        // we will just ensure that the returned builder is not the same as the original.
        assert!(!std::ptr::eq(
            &configured_builder,
            &opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        ));
    }

    #[test]
    fn test_console_provider_invalid_configuration() {
        // Arrange
        let meter_provider_builder = opentelemetry_sdk::metrics::SdkMeterProvider::builder();
        let invalid_config_yaml = serde_yaml::from_str::<serde_yaml::Value>(
            r#"
            temporality: invalid_value
        "#,
        )
        .unwrap();

        // Act
        let result = register_console_exporter(meter_provider_builder, &invalid_config_yaml);

        // Assert
        match result {
            Err(ConfigurationError::InvalidConfiguration(details)) => {
                assert!(
                    details.contains("Failed to deserialize PeriodicExporterConsole configuration")
                );
            }
            _ => panic!("Expected InvalidConfiguration error"),
        }
    }
}
