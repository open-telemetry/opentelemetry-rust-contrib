//! # Metrics reader provider module.
//!
//! This module provides providers for setting up metrics readers
//! in OpenTelemetry SDKs using declarative YAML configurations.

use opentelemetry_sdk::metrics::MeterProviderBuilder;
use serde_yaml::Value;

use crate::{model::metrics::reader::Reader, MetricsProvidersRegistry, ProviderError};

/// Provider for Metrics readers
pub struct ReaderProvider {
    periodic_reader_provider: PeriodicReaderProvider,
}

impl ReaderProvider {
    pub fn new() -> Self {
        ReaderProvider {
            periodic_reader_provider: PeriodicReaderProvider::default(),
        }
    }
    /// Configures a metrics reader based on the provided configuration
    pub fn configure(
        &self,
        metrics_registry: &MetricsProvidersRegistry,
        mut meter_provider_builder: MeterProviderBuilder,
        config: &Reader,
    ) -> Result<MeterProviderBuilder, ProviderError> {
        match config {
            crate::model::metrics::reader::Reader::Periodic(periodic_config) => {
                meter_provider_builder = self.periodic_reader_provider.configure(
                    metrics_registry,
                    meter_provider_builder,
                    periodic_config,
                )?;
            }
            crate::model::metrics::reader::Reader::Pull(_pull_config) => {
                // TODO: Implement pull reader configuration
            }
        }
        Ok(meter_provider_builder)
    }
}

impl Default for ReaderProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Periodic reader provider
pub struct PeriodicReaderProvider {
    periodic_exporter_provider: PeriodicExporterProvider,
}

impl PeriodicReaderProvider {
    /// Creates a new PeriodicReaderProvider
    pub fn new() -> Self {
        PeriodicReaderProvider {
            periodic_exporter_provider: PeriodicExporterProvider::default(),
        }
    }

    /// Configures a periodic metrics reader based on the provided configuration
    pub fn configure(
        &self,
        metrics_registry: &MetricsProvidersRegistry,
        mut meter_provider_builder: opentelemetry_sdk::metrics::MeterProviderBuilder,
        config: &crate::model::metrics::reader::Periodic,
    ) -> Result<opentelemetry_sdk::metrics::MeterProviderBuilder, ProviderError> {
        meter_provider_builder = self.periodic_exporter_provider.configure(
            metrics_registry,
            meter_provider_builder,
            &config.exporter,
        )?;
        Ok(meter_provider_builder)
    }
}

impl Default for PeriodicReaderProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Periodic exporter provider
pub struct PeriodicExporterProvider {}

impl PeriodicExporterProvider {
    /// Creates a new PeriodicExporterProvider
    pub fn new() -> Self {
        PeriodicExporterProvider {}
    }

    /// Configures a periodic metrics exporter based on the provided configuration
    pub fn configure(
        &self,
        metrics_registry: &MetricsProvidersRegistry,
        mut meter_provider_builder: opentelemetry_sdk::metrics::MeterProviderBuilder,
        config: &Value,
    ) -> Result<opentelemetry_sdk::metrics::MeterProviderBuilder, ProviderError> {
        match config.as_mapping() {
            Some(exporter_map) => {
                for key in exporter_map.keys() {
                    match key {
                        Value::String(exporter_name) => {
                            let exporter_factory_option =
                                metrics_registry.periodic_exporter_factory(&exporter_name);
                            match exporter_factory_option {
                                Some(factory_function) => {
                                    let config =
                                        &exporter_map[&Value::String(exporter_name.clone())];
                                    let meter_provider_builder_result =
                                        factory_function(meter_provider_builder, config);
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

impl Default for PeriodicExporterProvider {
    fn default() -> Self {
        Self::new()
    }
}

// Pull reader provider
pub struct PullReaderProvider {
    pull_exporter_provider: PullExporterProvider,
}

impl PullReaderProvider {
    /// Creates a new PullReaderProvider
    pub fn new() -> Self {
        PullReaderProvider {
            pull_exporter_provider: PullExporterProvider::default(),
        }
    }

    /// Configures a pull metrics reader based on the provided configuration
    pub fn configure(
        &self,
        metrics_registry: &MetricsProvidersRegistry,
        mut meter_provider_builder: opentelemetry_sdk::metrics::MeterProviderBuilder,
        config: &crate::model::metrics::reader::Pull,
    ) -> Result<opentelemetry_sdk::metrics::MeterProviderBuilder, ProviderError> {
        if let Some(exporter_config) = &config.exporter {
            meter_provider_builder = self.pull_exporter_provider.configure(
                metrics_registry,
                meter_provider_builder,
                exporter_config,
            )?;
        }
        Ok(meter_provider_builder)
    }
}

impl Default for PullReaderProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Pull exporter provider
pub struct PullExporterProvider {}

impl PullExporterProvider {
    /// Creates a new PullExporterProvider
    pub fn new() -> Self {
        PullExporterProvider {}
    }

    pub fn configure(
        &self,
        _metrics_registry: &MetricsProvidersRegistry,
        meter_provider_builder: opentelemetry_sdk::metrics::MeterProviderBuilder,
        config: &crate::model::metrics::reader::PullExporter,
    ) -> Result<opentelemetry_sdk::metrics::MeterProviderBuilder, ProviderError> {
        if let Some(_prometheus_config) = &config.prometheus {
            // Explicitly Prometheus exporter is not supported in this provider.
            return Err(ProviderError::UnsupportedExporter(
                "Prometheus exporter is not supported.".to_string(),
            ));
        }
        Ok(meter_provider_builder)
    }
}

impl Default for PullExporterProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::{model::metrics::reader::PullExporter, ConfigurationError};
    use opentelemetry_sdk::metrics::SdkMeterProvider;

    pub fn register_mock_exporter(
        builder: MeterProviderBuilder,
        _config: &serde_yaml::Value,
    ) -> Result<MeterProviderBuilder, ConfigurationError> {
        // Mock implementation: just return the builder as is
        Ok(builder)
    }

    #[test]
    fn test_reader_provider_configure() {
        let provider = ReaderProvider::default();
        let mut configuration_registry = crate::ConfigurationProvidersRegistry::new();
        configuration_registry
            .metrics_mut()
            .register_periodic_exporter_factory("console".to_string(), register_mock_exporter);
        let meter_provider_builder = SdkMeterProvider::builder();

        let console_object = serde_yaml::from_str(
            r#"
            console:
                temporality: cumulative
            "#,
        )
        .unwrap();
        let config = crate::model::metrics::reader::Reader::Periodic(
            crate::model::metrics::reader::Periodic {
                exporter: console_object,
            },
        );

        let metrics_registry = configuration_registry.metrics();

        _ = provider
            .configure(metrics_registry, meter_provider_builder, &config)
            .unwrap();
    }

    #[test]
    fn test_reader_provider_configure_console_factory_not_registered() {
        let provider = ReaderProvider::default();
        let metrics_registry = MetricsProvidersRegistry::new();
        let meter_provider_builder = SdkMeterProvider::builder();

        let console_config = serde_yaml::from_str(
            r#"
            console:
                temporality: cumulative
            "#,
        )
        .unwrap();

        let config = crate::model::metrics::reader::Reader::Periodic(
            crate::model::metrics::reader::Periodic {
                exporter: console_config,
            },
        );

        let result = provider.configure(&metrics_registry, meter_provider_builder, &config);
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
        let provider = ReaderProvider::new();
        let metrics_registry = MetricsProvidersRegistry::new();
        let meter_provider_builder = SdkMeterProvider::builder();

        let console_config = serde_yaml::from_str(
            r#"
            otlp:
              temporality: cumulative
            "#,
        )
        .unwrap();

        let config = crate::model::metrics::reader::Reader::Periodic(
            crate::model::metrics::reader::Periodic {
                exporter: console_config,
            },
        );

        let result = provider.configure(&metrics_registry, meter_provider_builder, &config);
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
        let provider = PullExporterProvider::new();
        let metrics_provider_manager = MetricsProvidersRegistry::new();
        let meter_provider_builder = SdkMeterProvider::builder();
        let config = crate::model::metrics::reader::PullExporter {
            prometheus: Some(crate::model::metrics::reader::PullExporterPrometheus {
                host: "localhost".to_string(),
                port: 9090,
            }),
        };
        let result = provider.configure(&metrics_provider_manager, meter_provider_builder, &config);
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
        let provider = PullReaderProvider::default();
        let configuration_registry = crate::ConfigurationProvidersRegistry::new();
        let meter_provider_builder = SdkMeterProvider::builder();

        let config = crate::model::metrics::reader::Pull {
            exporter: Some(PullExporter { prometheus: None }),
        };

        let metrics_registry = configuration_registry.metrics();

        _ = provider
            .configure(metrics_registry, meter_provider_builder, &config)
            .unwrap();
    }
}
