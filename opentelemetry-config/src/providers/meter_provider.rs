//! Provider for Metrics telemetry
//!
//! This module provides functionality to configure Metrics telemetry
//! in OpenTelemetry SDKs using declarative YAML configurations.
mod reader_provider;

use opentelemetry_sdk::metrics::MeterProviderBuilder;

use crate::{MeterProviderRegistry, ProviderError};

use crate::providers::meter_provider::reader_provider::ReaderProvider;

/// Provider for Metrics telemetry
pub(crate) struct MeterProvider {}

impl MeterProvider {
    /// Configures the Metrics provider based on the provided configuration
    pub(crate) fn configure(
        metrics_registry: &MeterProviderRegistry,
        mut meter_provider_builder: MeterProviderBuilder,
        config: &crate::model::metrics::Metrics,
    ) -> Result<MeterProviderBuilder, ProviderError> {
        for reader in &config.readers {
            meter_provider_builder =
                ReaderProvider::configure(metrics_registry, meter_provider_builder, reader)?;
        }

        Ok(meter_provider_builder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::metrics::Metrics;
    use opentelemetry_sdk::metrics::SdkMeterProvider;

    #[test]
    fn test_configure_metrics_provider() {
        let yaml_str = r#"
          readers:
            - periodic:
                exporter:
                  custom: {}
        "#;
        let metrics_config: Metrics = serde_yaml::from_str(yaml_str).unwrap();
        let mut registry = MeterProviderRegistry::default();

        fn mock_factory(
            builder: MeterProviderBuilder,
            _config: &str,
        ) -> Result<MeterProviderBuilder, crate::ConfigurationError> {
            Ok(builder)
        }

        let name = "custom";
        registry.register_exporter_factory(name, mock_factory);
        let meter_provider_builder = SdkMeterProvider::builder();
        let result = MeterProvider::configure(&registry, meter_provider_builder, &metrics_config);
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
        let registry = MeterProviderRegistry::default();
        let meter_provider_builder = SdkMeterProvider::builder();
        let result = MeterProvider::configure(&registry, meter_provider_builder, &metrics_config);
        match result {
            Err(ProviderError::NotRegisteredProvider(details)) => {
                println!("Error details: {}", details);
                assert!(details.contains("unknown_exporter"))
            }
            _ => panic!("Expected UnknownExporter error"),
        }
    }
}
