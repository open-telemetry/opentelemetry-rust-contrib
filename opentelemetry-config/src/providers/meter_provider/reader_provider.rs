//! # Metrics reader provider module.
//!
//! This module provides providers for setting up metrics readers
//! in OpenTelemetry SDKs using declarative YAML configurations.

use opentelemetry_sdk::metrics::{MeterProviderBuilder, PeriodicReader, Temporality};
use std::time::Duration;

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
            Reader::Pull(_) => Err(ProviderError::InvalidConfiguration(
                "Pull readers are not supported in this milestone".to_string(),
            )),
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
        let interval_ms = config.interval_ms();
        let interval_duration = Duration::from_millis(interval_ms);

        let exporter_map = config
            .exporter
            .as_ref()
            .and_then(|e| e.as_mapping())
            .ok_or_else(|| {
                ProviderError::InvalidConfiguration(
                    "Missing or invalid `exporter` in periodic reader".to_string(),
                )
            })?;

        let (key_val, val) = exporter_map.iter().next().unwrap();
        let exporter_name = key_val.as_str().ok_or_else(|| {
            ProviderError::InvalidConfiguration("Exporter name must be a string".to_string())
        })?;

        match exporter_name {
            "console" => {
                let mut temporality = None;
                if let Some(console_map) = val.as_mapping() {
                    if let Some(temp_val) = console_map.get("temporality_preference") {
                        if let Some(temp_str) = temp_val.as_str() {
                            match temp_str {
                                "cumulative" => temporality = Some(Temporality::Cumulative),
                                "delta" => temporality = Some(Temporality::Delta),
                                "low_memory" => temporality = Some(Temporality::LowMemory),
                                _ => {
                                    return Err(ProviderError::InvalidConfiguration(format!(
                                        "Unsupported temporality preference '{}' for console exporter",
                                        temp_str
                                    )));
                                }
                            }
                        }
                    }
                }

                let mut exporter_builder = opentelemetry_stdout::MetricExporter::builder();
                if let Some(t) = temporality {
                    exporter_builder = exporter_builder.with_temporality(t);
                }
                let exporter = exporter_builder.build();

                let reader = PeriodicReader::builder(exporter)
                    .with_interval(interval_duration)
                    .build();

                Ok(meter_provider_builder.with_reader(reader))
            }
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
    use opentelemetry_sdk::metrics::SdkMeterProvider;

    pub fn register_mock_exporter_factory(
        builder: MeterProviderBuilder,
        _config: &str,
    ) -> Result<MeterProviderBuilder, ConfigurationError> {
        Ok(builder)
    }

    #[test]
    fn test_reader_provider_configure_console() {
        let registry = MeterProviderRegistry::default();
        let meter_provider_builder = SdkMeterProvider::builder();

        let config: Reader = serde_yaml::from_str(
            r#"
            periodic:
                interval: 5000
                exporter:
                    console: {}
            "#,
        )
        .unwrap();

        let result = ReaderProvider::configure(&registry, meter_provider_builder, &config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_reader_provider_configure_console_with_temporality() {
        let registry = MeterProviderRegistry::default();
        let meter_provider_builder = SdkMeterProvider::builder();

        let config: Reader = serde_yaml::from_str(
            r#"
            periodic:
                interval: 5000
                exporter:
                    console:
                        temporality_preference: delta
            "#,
        )
        .unwrap();

        let result = ReaderProvider::configure(&registry, meter_provider_builder, &config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_reader_provider_configure_custom_registered() {
        let mut registry = MeterProviderRegistry::default();
        let name = "my_custom_exporter";
        registry.register_exporter_factory(name, register_mock_exporter_factory);
        let meter_provider_builder = SdkMeterProvider::builder();

        let config: Reader = serde_yaml::from_str(
            r#"
            periodic:
                interval: 60000
                exporter:
                    my_custom_exporter:
                        custom_field: "test"
            "#,
        )
        .unwrap();

        let result = ReaderProvider::configure(&registry, meter_provider_builder, &config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_reader_provider_configure_custom_not_registered() {
        let registry = MeterProviderRegistry::default();
        let meter_provider_builder = SdkMeterProvider::builder();

        let config: Reader = serde_yaml::from_str(
            r#"
            periodic:
                interval: 60000
                exporter:
                    unregistered_custom: {}
            "#,
        )
        .unwrap();

        let result = ReaderProvider::configure(&registry, meter_provider_builder, &config);
        match result {
            Err(ProviderError::NotRegisteredProvider(msg)) => {
                assert!(msg.contains("unregistered_custom"));
            }
            other => panic!("Expected NotRegisteredProvider, got {:?}", other),
        }
    }

    #[test]
    fn test_reader_provider_configure_unsupported_standard_exporter() {
        let registry = MeterProviderRegistry::default();
        let meter_provider_builder = SdkMeterProvider::builder();

        let config: Reader = serde_yaml::from_str(
            r#"
            periodic:
                exporter:
                    otlp_http:
                        endpoint: "http://localhost:4318"
            "#,
        )
        .unwrap();

        let result = ReaderProvider::configure(&registry, meter_provider_builder, &config);
        match result {
            Err(ProviderError::UnsupportedExporter(msg)) => {
                assert!(msg.contains("otlp_http"));
            }
            other => panic!("Expected UnsupportedExporter, got {:?}", other),
        }
    }

    #[test]
    fn test_pull_reader_rejected() {
        let registry = MeterProviderRegistry::default();
        let meter_provider_builder = SdkMeterProvider::builder();

        let config: Reader = serde_yaml::from_str(
            r#"
            pull:
                exporter:
                    prometheus: {}
            "#,
        )
        .unwrap();

        let result = ReaderProvider::configure(&registry, meter_provider_builder, &config);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Pull readers are not supported"));
    }

    #[test]
    fn test_timeout_rejected() {
        let config: Reader = serde_yaml::from_str(
            r#"
            periodic:
                interval: 1000
                timeout: 500
                exporter:
                    console: {}
            "#,
        )
        .unwrap();

        let result = config.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("timeout is not supported"));
    }
}
