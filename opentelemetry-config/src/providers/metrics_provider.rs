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

    /// Configures the Metrics provider based on the provided configuration
    pub fn configure(
        &self,
        metrics_registry: &MetricsProvidersRegistry,
        mut meter_provider_builder: MeterProviderBuilder,
        config: &crate::model::metrics::Metrics,
    ) -> Result<MeterProviderBuilder, ProviderError> {
        for reader in &config.readers {
            meter_provider_builder =
                self.reader_provider
                    .configure(metrics_registry, meter_provider_builder, reader)?;
        }

        Ok(meter_provider_builder)
    }
}

impl Default for MetricsProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::metrics::Metrics;
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use serde_yaml;

    #[test]
    fn test_configure_metrics_provider() {
        let yaml_str = r#"
          readers:
            - periodic:
                exporter:
                  custom: {}
        "#;
        let metrics_config: Metrics = serde_yaml::from_str(yaml_str).unwrap();
        let mut registry = MetricsProvidersRegistry::new();

        fn mock_factory(
            builder: MeterProviderBuilder,
            _config: &serde_yaml::Value,
        ) -> Result<MeterProviderBuilder, crate::ConfigurationError> {
            Ok(builder)
        }

        registry.register_periodic_exporter_factory("custom".to_string(), mock_factory);
        let meter_provider_builder = SdkMeterProvider::builder();
        let metrics_provider = MetricsProvider::new();
        let result = metrics_provider.configure(&registry, meter_provider_builder, &metrics_config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_configure_metrics_provider_with_unknown_exporter() {
        let yaml_str = r#"
          readers:
            - periodic:
                exporter:
                  unknown_exporter: {}
        "#;
        let metrics_config: Metrics = serde_yaml::from_str(yaml_str).unwrap();
        let registry = MetricsProvidersRegistry::default();
        let meter_provider_builder = SdkMeterProvider::builder();
        let metrics_provider = MetricsProvider::new();
        let result = metrics_provider.configure(&registry, meter_provider_builder, &metrics_config);
        match result {
            Err(ProviderError::NotRegisteredProvider(details)) => {
                println!("Error details: {}", details);
                assert!(details.contains("unknown_exporter"))
            }
            _ => panic!("Expected UnknownExporter error"),
        }
    }
}
