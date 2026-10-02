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
        if let Some(readers) = &config.readers {
            for reader in readers {
                meter_provider_builder =
                    ReaderProvider::configure(metrics_registry, meter_provider_builder, reader)?;
            }
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
        let metrics_config: MeterProviderConfig = serde_yaml::from_str(yaml_str).unwrap();
        let registry = MeterProviderRegistry::default();
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
        let metrics_config: MeterProviderConfig = serde_yaml::from_str(yaml_str).unwrap();
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
        let metrics_config: MeterProviderConfig = serde_yaml::from_str(yaml_str).unwrap();
        let result = metrics_config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("views"));
    }
}
