//! # Metrics reader provider module.
//!
//! This module provides providers for setting up metrics readers
//! in OpenTelemetry SDKs using declarative YAML configurations.

use opentelemetry_sdk::metrics::MeterProviderBuilder;

use crate::{
    model::metrics::reader::{PeriodicReaderConfig, Reader},
    MeterProviderRegistry, ProviderError,
};

/// Provider for Metrics readers
pub(crate) struct ReaderProvider {}

impl ReaderProvider {
    /// Configures a metrics reader based on the provided configuration
    pub(crate) fn configure(
        metrics_registry: &MeterProviderRegistry,
        meter_provider_builder: MeterProviderBuilder,
        config: &Reader,
    ) -> Result<MeterProviderBuilder, ProviderError> {
        match config {
            Reader::Periodic(periodic_config) => PeriodicReaderProvider::configure(
                metrics_registry,
                meter_provider_builder,
                periodic_config,
            ),
        }
    }
}

/// Periodic reader provider
struct PeriodicReaderProvider {}

impl PeriodicReaderProvider {
    /// Configures a periodic metrics reader based on the provided configuration
    fn configure(
        metrics_registry: &MeterProviderRegistry,
        meter_provider_builder: MeterProviderBuilder,
        config: &PeriodicReaderConfig,
    ) -> Result<MeterProviderBuilder, ProviderError> {
        let exporter_map = config
            .exporter
            .as_ref()
            .and_then(|e| e.as_mapping())
            .ok_or_else(|| {
                ProviderError::InvalidConfiguration(
                    "Missing or invalid `exporter` in periodic reader".to_string(),
                )
            })?;

        let (key_val, _) = exporter_map.iter().next().unwrap();
        let exporter_name = key_val.as_str().ok_or_else(|| {
            ProviderError::InvalidConfiguration("Exporter name must be a string".to_string())
        })?;

        match exporter_name {
            "otlp_http" | "otlp_grpc" | "otlp_file/development" | "otlp_file"
            | "prometheus/development" | "prometheus" => {
                Err(ProviderError::UnsupportedExporter(format!(
                    "Exporter '{}' is not supported in this milestone",
                    exporter_name
                )))
            }
            custom_exporter_name => {
                match metrics_registry.provider_factory(custom_exporter_name) {
                    Some(factory_function) => {
                        let periodic_config_str = &config.raw_yaml;
                        let updated_builder = factory_function(
                            meter_provider_builder,
                            periodic_config_str,
                        ).map_err(|e| match e {
                            crate::ConfigurationError::InvalidConfiguration(msg) => {
                                ProviderError::InvalidConfiguration(msg)
                            }
                            crate::ConfigurationError::RegistrationError(msg) => {
                                ProviderError::RegistrationError(msg)
                            }
                        })?;
                        Ok(updated_builder)
                    }
                    None => Err(ProviderError::NotRegisteredProvider(format!(
                        "No provider found for periodic exporter '{}'. Make sure it is registered with its factory.",
                        custom_exporter_name
                    ))),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::ConfigurationError;
    fn parse(yaml: &str) -> Result<Reader, ProviderError> {
        Reader::from_value(&serde_yaml::from_str(yaml).unwrap())
    }
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

        let config = parse(
            r#"
            periodic:
                interval: 60000
                exporter:
                    console:
                        temporality_preference: cumulative
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

        let config = parse(
            r#"
            periodic:
                interval: 60000
                exporter:
                    console:
                        temporality_preference: cumulative
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

        let config = parse(
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
}
