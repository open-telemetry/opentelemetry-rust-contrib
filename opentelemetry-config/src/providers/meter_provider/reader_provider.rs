//! # Metrics reader provider module.
//!
//! This module provides providers for setting up metrics readers
//! in OpenTelemetry SDKs using declarative YAML configurations.

use opentelemetry_sdk::metrics::MeterProviderBuilder;
use serde_yaml::Value;

use crate::{model::metrics::reader::Reader, MeterProviderRegistry, ProviderError};

/// Provider for Metrics readers
pub(crate) struct ReaderProvider {}

impl ReaderProvider {
    /// Configures a metrics reader based on the provided configuration
    pub(crate) fn configure(
        metrics_registry: &MeterProviderRegistry,
        mut meter_provider_builder: MeterProviderBuilder,
        config: &Reader,
    ) -> Result<MeterProviderBuilder, ProviderError> {
        match config {
            crate::model::metrics::reader::Reader::Periodic(periodic_config) => {
                meter_provider_builder = PeriodicReaderProvider::configure(
                    metrics_registry,
                    meter_provider_builder,
                    periodic_config,
                )?;
            }
            crate::model::metrics::reader::Reader::Pull(pull_config) => {
                meter_provider_builder = PullReaderProvider::configure(
                    metrics_registry,
                    meter_provider_builder,
                    pull_config,
                )?;
            }
        }
        Ok(meter_provider_builder)
    }
}

/// Periodic reader provider
struct PeriodicReaderProvider {}

impl PeriodicReaderProvider {
    /// Configures a periodic metrics reader based on the provided configuration
    fn configure(
        metrics_registry: &MeterProviderRegistry,
        mut meter_provider_builder: opentelemetry_sdk::metrics::MeterProviderBuilder,
        config: &Value,
    ) -> Result<opentelemetry_sdk::metrics::MeterProviderBuilder, ProviderError> {
        meter_provider_builder =
            PeriodicExporterProvider::configure(metrics_registry, meter_provider_builder, config)?;
        Ok(meter_provider_builder)
    }
}

/// Periodic exporter provider
struct PeriodicExporterProvider {}

impl PeriodicExporterProvider {
    /// Configures a periodic metrics exporter based on the provided configuration
    fn configure(
        metrics_registry: &MeterProviderRegistry,
        mut meter_provider_builder: opentelemetry_sdk::metrics::MeterProviderBuilder,
        periodic_config: &Value,
    ) -> Result<opentelemetry_sdk::metrics::MeterProviderBuilder, ProviderError> {
        let config = &periodic_config["exporter"];
        match config.as_mapping() {
            Some(exporter_map) => {
                for key in exporter_map.keys() {
                    match key {
                        Value::String(exporter_name) => {
                            let reader_factory_option =
                                metrics_registry.provider_factory(exporter_name);
                            match reader_factory_option {
                                Some(factory_function) => {
                                    let periodic_config_str =
                                        serde_yaml::to_string(&periodic_config).map_err(|e| {
                                            ProviderError::RegistrationError(format!(
                                                "Failed to serialize periodic configuration: {}",
                                                e
                                            ))
                                        })?;

                                    let meter_provider_builder_result = factory_function(
                                        meter_provider_builder,
                                        &periodic_config_str,
                                    );
                                    meter_provider_builder = match meter_provider_builder_result {
                                        Ok(builder) => builder,
                                        Err(e) => match e {
                                            crate::ConfigurationError::InvalidConfiguration(
                                                msg,
                                            ) => {
                                                return Err(ProviderError::InvalidConfiguration(
                                                    msg,
                                                ));
                                            }
                                            crate::ConfigurationError::RegistrationError(msg) => {
                                                return Err(ProviderError::RegistrationError(msg));
                                            }
                                        },
                                    };
                                }
                                None => {
                                    return Err(ProviderError::NotRegisteredProvider(format!(
                                        "No provider found for periodic exporter '{}'. Make sure it is registered with its factory.",
                                        exporter_name
                                    )));
                                }
                            }
                        }
                        _ => {
                            return Err(ProviderError::InvalidConfiguration(
                                "Exporter name must be a string.".to_string(),
                            ));
                        }
                    }
                }
            }
            None => {
                return Err(ProviderError::InvalidConfiguration(
                    "Expecting a configuration object for periodic exporter.".to_string(),
                ));
            }
        }
        Ok(meter_provider_builder)
    }
}

// Pull reader provider
struct PullReaderProvider {}

impl PullReaderProvider {
    /// Configures a pull metrics reader based on the provided configuration
    fn configure(
        metrics_registry: &MeterProviderRegistry,
        mut meter_provider_builder: opentelemetry_sdk::metrics::MeterProviderBuilder,
        config: &Value,
    ) -> Result<opentelemetry_sdk::metrics::MeterProviderBuilder, ProviderError> {
        if let Some(exporter_config) = config["exporter"].as_mapping() {
            meter_provider_builder = PullExporterProvider::configure(
                metrics_registry,
                meter_provider_builder,
                &Value::Mapping(exporter_config.clone()),
            )?;
        }
        Ok(meter_provider_builder)
    }
}

/// Pull exporter provider
struct PullExporterProvider {}

impl PullExporterProvider {
    /// Configures a pull metrics exporter based on the provided configuration
    fn configure(
        _metrics_registry: &MeterProviderRegistry,
        meter_provider_builder: opentelemetry_sdk::metrics::MeterProviderBuilder,
        config: &Value,
    ) -> Result<opentelemetry_sdk::metrics::MeterProviderBuilder, ProviderError> {
        // TODO: Implement other pull exporters.
        if let Some(_prometheus_config) = config["prometheus"].as_mapping() {
            // Explicitly Prometheus exporter is not supported in this provider.
            return Err(ProviderError::UnsupportedExporter(
                "Prometheus exporter is not supported.".to_string(),
            ));
        }
        Ok(meter_provider_builder)
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::ConfigurationError;
    use opentelemetry_sdk::metrics::SdkMeterProvider;

    pub fn register_mock_exporter_factory(
        builder: MeterProviderBuilder,
        _config: &str,
    ) -> Result<MeterProviderBuilder, ConfigurationError> {
        // Mock implementation: just return the builder as is
        Ok(builder)
    }

    #[test]
    fn test_reader_provider_configure() {
        let mut registry = crate::ConfigurationProviderRegistry::default();
        let name = "console";
        registry.register_metric_exporter_factory(name, register_mock_exporter_factory);
        let meter_provider_builder = SdkMeterProvider::builder();

        let config: Reader = serde_yaml::from_str(
            r#"
            periodic:
                interval: 60000
                exporter:
                    console:
                        temporality: cumulative
            "#,
        )
        .unwrap();

        let metrics_registry = registry.metrics;

        _ = ReaderProvider::configure(&metrics_registry, meter_provider_builder, &config).unwrap();
    }

    #[test]
    fn test_reader_provider_configure_console_factory_not_registered() {
        let metrics_registry = MeterProviderRegistry::default();
        let meter_provider_builder = SdkMeterProvider::builder();

        let config: Reader = serde_yaml::from_str(
            r#"
            periodic:
                interval: 60000
                exporter:
                    console:
                        temporality: cumulative
            "#,
        )
        .unwrap();

        let result = ReaderProvider::configure(&metrics_registry, meter_provider_builder, &config);
        if let Err(e) = result {
            println!("Error: {}", e);
            assert!(e
                .to_string()
                .contains("No provider found for periodic exporter 'console'"));
        } else {
            panic!("Expected error due to missing provider, but got Ok");
        }
    }

    #[test]
    fn test_reader_provider_provide_otlp_factory_not_registered() {
        let metrics_registry = MeterProviderRegistry::default();
        let meter_provider_builder = SdkMeterProvider::builder();

        let config: Reader = serde_yaml::from_str(
            r#"
            periodic:
                interval: 60000
                exporter:
                    otlp:
                        protocol: http/protobuf
                        endpoint: http://localhost:4317
            "#,
        )
        .unwrap();

        let result = ReaderProvider::configure(&metrics_registry, meter_provider_builder, &config);
        if let Err(e) = result {
            assert!(e
                .to_string()
                .contains("No provider found for periodic exporter 'otlp'"));
        } else {
            panic!("Expected error due to missing provider, but got Ok");
        }
    }

    #[test]
    fn test_periodic_exporter_provider_configure_unsupported_exporter() {
        let metrics_provider_manager = MeterProviderRegistry::default();
        let meter_provider_builder = SdkMeterProvider::builder();
        let confg_yaml = r#"
            prometheus:
                host: localhost
                port: 9090
        "#;
        let config: Value = serde_yaml::from_str(confg_yaml).unwrap();
        let result = PullExporterProvider::configure(
            &metrics_provider_manager,
            meter_provider_builder,
            &config,
        );
        if let Err(e) = result {
            assert!(e
                .to_string()
                .contains("Prometheus exporter is not supported."));
        } else {
            panic!("Expected error due to unsupported exporter, but got Ok");
        }
    }

    #[test]
    fn test_pull_reader_provider_configure_basic() {
        let configuration_registry = crate::ConfigurationProviderRegistry::default();
        let meter_provider_builder = SdkMeterProvider::builder();

        let config_yaml = "";
        let config: Value = serde_yaml::from_str(config_yaml).unwrap();

        let metrics_registry = configuration_registry.metrics;

        _ = PullReaderProvider::configure(&metrics_registry, meter_provider_builder, &config)
            .unwrap();
    }
}
