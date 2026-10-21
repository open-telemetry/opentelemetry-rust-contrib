//! Provider for Metrics telemetry
//!
//! This module provides functionality to configure Metrics telemetry
//! in OpenTelemetry SDKs using declarative YAML configurations.
mod reader_provider;

use opentelemetry_sdk::metrics::MeterProviderBuilder;

use crate::{model::metrics::MeterProviderConfig, MeterProviderRegistry, ProviderError};

use crate::providers::meter_provider::reader_provider::ReaderProvider;

/// Provider for Metrics telemetry
pub(crate) struct MeterProvider {}

impl MeterProvider {
    /// Configures the Metrics provider based on the provided configuration
    pub(crate) fn configure(
        metrics_registry: &MeterProviderRegistry,
        mut meter_provider_builder: MeterProviderBuilder,
        config: &MeterProviderConfig,
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
    use crate::model::metrics::MeterProviderConfig;
    use opentelemetry_sdk::metrics::SdkMeterProvider;

    #[test]
    fn test_configure_metrics_provider() {
        let yaml_str = r#"
          readers:
            - periodic:
                exporter:
                  console: {}
        "#;
        let value = serde_yaml::from_str(yaml_str).unwrap();
        let metrics_config = MeterProviderConfig::from_value(&value).unwrap();
        let mut registry = MeterProviderRegistry::default();
        registry.register_exporter_factory("console", |builder, _| Ok(builder));
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
        let value = serde_yaml::from_str(yaml_str).unwrap();
        let metrics_config = MeterProviderConfig::from_value(&value).unwrap();
        let registry = MeterProviderRegistry::default();
        let meter_provider_builder = SdkMeterProvider::builder();
        let result = MeterProvider::configure(&registry, meter_provider_builder, &metrics_config);
        match result {
            Err(ProviderError::NotRegisteredProvider(details)) => {
                assert!(details.contains("unknown_exporter"));
            }
            _ => panic!("Expected NotRegisteredProvider error"),
        }
    }

    #[test]
    fn test_configure_metrics_provider_with_unsupported_views() {
        let yaml_str = r#"
          readers:
            - periodic:
                exporter:
                  console: {}
          views:
            - selector:
                instrument_name: "test"
              stream:
                name: "test_renamed"
        "#;
        let value = serde_yaml::from_str(yaml_str).unwrap();
        let result = MeterProviderConfig::from_value(&value);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("views"));
    }
}
