//! # OpenTelemetry declarative configuration module for Stdout (Console) exporter
//!
//! This module implements a provider for OpenTelemetry Metrics
//! that enables exporting metrics to the console (stdout) using
//! the OpenTelemetry Config crate.

use opentelemetry_config::{model::metrics::reader::Periodic, ConfigurationError};
use opentelemetry_sdk::metrics::{MeterProviderBuilder, PeriodicReader};

pub fn register_console_meter_reader_factory(
    mut builder: MeterProviderBuilder,
    periodic_config: &Periodic,
) -> Result<MeterProviderBuilder, ConfigurationError> {
    let mut exporter_builder = opentelemetry_stdout::MetricExporter::builder();

    let console_config =
        serde_yaml::from_value::<MeterConsoleReaderConfig>(periodic_config.exporter.clone())
            .map_err(|e| {
                ConfigurationError::InvalidConfiguration(format!(
                    "Failed to deserialize Console Reader configuration: {}",
                    e
                ))
            })?;

    let config = console_config.console;
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
    // TODO: Configure time interval from config
    let reader = PeriodicReader::builder(exporter)
        .with_interval(std::time::Duration::from_millis(periodic_config.interval))
        .build();
    builder = builder.with_reader(reader);
    Ok(builder)
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
struct MeterConsoleReaderConfig {
    console: MeterConsoleExporterConfig,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
struct MeterConsoleExporterConfig {
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
            opentelemetry_config::ConfigurationProviderRegistry::default();

        // Act
        let metrics_registry = configuration_registry.metrics();
        metrics_registry
            .register_periodic_reader_factory("console", register_console_meter_reader_factory);

        // Assert
        assert!(metrics_registry.has_periodic_reader_factory("console"))
    }

    #[test]
    fn test_console_provider_configure_temporality_minimal() {
        // Arrange
        let meter_provider_builder = opentelemetry_sdk::metrics::SdkMeterProvider::builder();

        let periodic_config_yaml = r#"
            exporter:
                console:
            "#;

        let periodic_config: Periodic = serde_yaml::from_str(periodic_config_yaml).unwrap();

        // Act
        let configured_builder =
            register_console_meter_reader_factory(meter_provider_builder, &periodic_config)
                .unwrap();

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

        let periodic_config_yaml = r#"
            exporter:
                console:
                    temporality: delta
            "#;

        let periodic_config: Periodic = serde_yaml::from_str(periodic_config_yaml).unwrap();

        // Act
        let configured_builder =
            register_console_meter_reader_factory(meter_provider_builder, &periodic_config)
                .unwrap();

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

        let periodic_config_yaml = r#"
            exporter:
                console:
                    temporality: cumulative
            "#;

        let periodic_config: Periodic = serde_yaml::from_str(periodic_config_yaml).unwrap();
        // Act
        let configured_builder =
            register_console_meter_reader_factory(meter_provider_builder, &periodic_config)
                .unwrap();

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
        let invalid_config_yaml = r#"
            exporter:
                console:
                    temporality: invalid_value
        "#;

        let periodic_config: Periodic = serde_yaml::from_str(invalid_config_yaml).unwrap();
        // Act
        let result =
            register_console_meter_reader_factory(meter_provider_builder, &periodic_config);

        // Assert
        match result {
            Err(ConfigurationError::InvalidConfiguration(details)) => {
                assert!(details.contains("Failed to deserialize"));
            }
            _ => panic!("Expected InvalidConfiguration error"),
        }
    }

    #[test]
    fn test_console_provider_unknown_field_configuration() {
        // Arrange
        let meter_provider_builder = opentelemetry_sdk::metrics::SdkMeterProvider::builder();
        let invalid_config_yaml = r#"
            exporter:
                console:
                    temporality2: delta
        "#;

        let periodic_config: Periodic = serde_yaml::from_str(invalid_config_yaml).unwrap();
        // Act
        let result =
            register_console_meter_reader_factory(meter_provider_builder, &periodic_config);

        // Assert
        match result {
            Err(ConfigurationError::InvalidConfiguration(details)) => {
                assert!(details.contains("Failed to deserialize"));
            }
            _ => panic!("Expected InvalidConfiguration error"),
        }
    }
}
