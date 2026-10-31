//! # Metrics reader provider module.
//!
//! This module provides providers for setting up metrics readers
//! in OpenTelemetry SDKs using declarative YAML configurations.

use opentelemetry_sdk::metrics::{MeterProviderBuilder, PeriodicReader, Temporality};
use std::time::Duration;

use crate::{
    model::metrics::reader::{is_standard_exporter, PeriodicReaderConfig, Reader},
    MeterProviderRegistry, ProviderError,
};

/// Provider for Metrics readers
pub(crate) struct ReaderProvider {}

impl ReaderProvider {
    /// Configures a metrics reader based on the provided configuration
    pub(crate) fn configure(
        metrics_registry: &MeterProviderRegistry,
        meter_provider_builder: &mut MeterProviderBuilder,
        config: &Reader,
    ) -> Result<(), ProviderError> {
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
        meter_provider_builder: &mut MeterProviderBuilder,
        config: &PeriodicReaderConfig,
    ) -> Result<(), ProviderError> {
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

                *meter_provider_builder = std::mem::take(meter_provider_builder).with_reader(reader);
                Ok(())
            }
            name if is_standard_exporter(name) => {
                Err(ProviderError::UnsupportedExporter(format!(
                    "Exporter '{}' is not supported in this milestone",
                    exporter_name
                )))
            }
            custom_exporter_name => {
                match metrics_registry.provider_factory(custom_exporter_name) {
                    Some(factory_function) => factory_function(
                        meter_provider_builder,
                        interval_duration,
                        val,
                    )
                    .map_err(|e| match e {
                        crate::ConfigurationError::InvalidConfiguration(msg) => {
                            ProviderError::InvalidConfiguration(msg)
                        }
                        crate::ConfigurationError::RegistrationError(msg) => {
                            ProviderError::RegistrationError(msg)
                        }
                    }),
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

    fn parse(yaml: &str) -> Result<Reader, ProviderError> {
        Reader::from_value(&serde_yaml::from_str(yaml).unwrap())
    }

    pub fn register_mock_exporter_factory(
        _config: &serde_yaml::Value,
    ) -> Result<opentelemetry_stdout::MetricExporter, ConfigurationError> {
        Ok(opentelemetry_stdout::MetricExporter::default())
    }

    #[test]
    fn test_reader_provider_configure_console() {
        let registry = MeterProviderRegistry::default();
        let mut meter_provider_builder = SdkMeterProvider::builder();

        let config = parse(
            r#"
            periodic:
                interval: 5000
                exporter:
                    console: {}
            "#,
        )
        .unwrap();

        let result = ReaderProvider::configure(&registry, &mut meter_provider_builder, &config);
        result.unwrap();
        meter_provider_builder.build().shutdown().unwrap();
    }

    #[test]
    fn test_reader_provider_configure_console_with_temporality() {
        let registry = MeterProviderRegistry::default();
        let mut meter_provider_builder = SdkMeterProvider::builder();

        let config = parse(
            r#"
            periodic:
                interval: 5000
                exporter:
                    console:
                        temporality_preference: delta
            "#,
        )
        .unwrap();

        let result = ReaderProvider::configure(&registry, &mut meter_provider_builder, &config);
        result.unwrap();
        meter_provider_builder.build().shutdown().unwrap();
    }

    #[test]
    fn test_reader_provider_configure_custom_registered() {
        let mut registry = MeterProviderRegistry::default();
        let name = "my_custom_exporter";
        registry
            .register_exporter_factory(name, register_mock_exporter_factory)
            .unwrap();
        let mut meter_provider_builder = SdkMeterProvider::builder();

        let config = parse(
            r#"
            periodic:
                interval: 60000
                exporter:
                    my_custom_exporter:
                        custom_field: "test"
            "#,
        )
        .unwrap();

        let result = ReaderProvider::configure(&registry, &mut meter_provider_builder, &config);
        result.unwrap();
        meter_provider_builder.build().shutdown().unwrap();
    }

    #[test]
    fn test_reader_provider_configure_custom_not_registered() {
        let registry = MeterProviderRegistry::default();
        let mut meter_provider_builder = SdkMeterProvider::builder();

        let config = parse(
            r#"
            periodic:
                interval: 60000
                exporter:
                    unregistered_custom: {}
            "#,
        )
        .unwrap();

        let result = ReaderProvider::configure(&registry, &mut meter_provider_builder, &config);
        match result {
            Err(ProviderError::NotRegisteredProvider(msg)) => {
                assert!(msg.contains("unregistered_custom"));
            }
            other => panic!("Expected NotRegisteredProvider, got {:?}", other),
        }
    }

    #[test]
    fn test_reader_parser_rejects_unsupported_standard_exporter() {
        let result = parse("periodic:\n  exporter:\n    otlp_http: {}\n");
        match result {
            Err(ProviderError::UnsupportedExporter(msg)) => assert!(msg.contains("otlp_http")),
            other => panic!("Expected UnsupportedExporter, got {:?}", other),
        }
    }

    #[test]
    fn test_pull_reader_rejected() {
        let err = parse("pull:\n  exporter:\n    prometheus: {}\n").unwrap_err();
        assert!(err.to_string().contains("Pull readers are not supported"));
    }

    #[test]
    fn test_timeout_rejected() {
        let err = parse("periodic:\n  timeout: 500\n  exporter: {console: {}}\n").unwrap_err();
        assert!(err.to_string().contains("timeout is not supported"));
    }
}
