//! # Provider objects for OpenTelemetry SDKs
//!
//! This module provides the different element providers to configure
//! OpenTelemetry SDKs using declarative YAML configurations.

mod meter_provider;

use opentelemetry_sdk::{
    logs::SdkLoggerProvider, metrics::SdkMeterProvider, trace::SdkTracerProvider, Resource,
};

use crate::{
    model::Telemetry, providers::meter_provider::MeterProvider, ConfigurationProviderRegistry,
    ProviderError,
};

/// Holds the configured telemetry providers
#[derive(Debug)]
pub struct TelemetryProviders {
    meter_provider: Option<SdkMeterProvider>,
    logs_provider: Option<SdkLoggerProvider>,
    traces_provider: Option<SdkTracerProvider>,
}

impl TelemetryProviders {
    fn new(
        meter_provider: Option<SdkMeterProvider>,
        logs_provider: Option<SdkLoggerProvider>,
        traces_provider: Option<SdkTracerProvider>,
    ) -> Self {
        TelemetryProviders {
            meter_provider,
            logs_provider,
            traces_provider,
        }
    }

    /// Returns a reference to the configured MeterProvider, if any
    pub fn meter_provider(&self) -> Option<&SdkMeterProvider> {
        self.meter_provider.as_ref()
    }

    /// Returns a reference to the configured LoggerProvider, if any
    pub fn logs_provider(&self) -> Option<&SdkLoggerProvider> {
        self.logs_provider.as_ref()
    }

    /// Returns a reference to the configured TracerProvider, if any
    pub fn traces_provider(&self) -> Option<&SdkTracerProvider> {
        self.traces_provider.as_ref()
    }

    /// Configures the Telemetry providers based on the provided configuration
    pub(crate) fn configure(
        configuration_registry: &ConfigurationProviderRegistry,
        config: &Telemetry,
    ) -> Result<TelemetryProviders, ProviderError> {
        config.validate()?;

        let resource: Resource = match &config.resource {
            Some(res_config) => res_config.to_resource()?,
            None => crate::model::resource::ResourceConfig::default().to_resource()?,
        };

        let meter_provider_option: Option<SdkMeterProvider>;
        if let Some(meter_provider_config) = &config.meter_provider {
            let mut meter_provider_builder = SdkMeterProvider::builder().with_resource(resource);
            meter_provider_builder = MeterProvider::configure(
                &configuration_registry.metrics,
                meter_provider_builder,
                meter_provider_config,
            )?;
            let meter_provider = meter_provider_builder.build();
            meter_provider_option = Some(meter_provider);
        } else {
            meter_provider_option = None;
        }

        // TODO: Add logs configuration
        let logs_provider_option = None;

        // TODO: Add traces configuration
        let traces_provider_option = None;

        let providers = TelemetryProviders::new(
            meter_provider_option,
            logs_provider_option,
            traces_provider_option,
        );

        Ok(providers)
    }

    /// Configures the Telemetry providers from a YAML string
    pub fn configure_from_yaml_str(
        configuration_registry: &ConfigurationProviderRegistry,
        yaml_str: &str,
    ) -> Result<TelemetryProviders, ProviderError> {
        let value: serde_yaml::Value = serde_yaml::from_str(yaml_str).map_err(|e| {
            ProviderError::InvalidConfiguration(format!(
                "Failed to parse YAML configuration: {}",
                e
            ))
        })?;
        let config = crate::model::Telemetry::from_value(&value)?;
        Self::configure(configuration_registry, &config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_configure_telemetry_from_yaml() {
        let yaml_str = r#"
        file_format: "1.2"
        meter_provider:
          readers:
            - periodic:
                exporter:
                  console:
                    temporality_preference: delta
        resource:
          attributes:
            - name: service.name
              value: "test-service"
            - name: service.version
              value: "1.0.0"
            - name: replica.count
              value: 3
              type: int
            - name: cores
              value: 4.5
              type: double
            - name: development
              value: true
              type: bool
        "#;

        let registry = ConfigurationProviderRegistry::default();
        let providers = TelemetryProviders::configure_from_yaml_str(&registry, yaml_str).unwrap();
        assert!(providers.meter_provider().is_some());
    }

    #[test]
    fn test_telemetry_provider_without_meter_provider() {
        let configuration_registry = ConfigurationProviderRegistry::default();
        let telemetry = Telemetry {
            file_format: Some("1.2".to_string()),
            resource: None,
            meter_provider: None,
            ..Default::default()
        };
        let providers = TelemetryProviders::configure(&configuration_registry, &telemetry).unwrap();
        assert!(providers.meter_provider().is_none());
    }

    #[test]
    fn test_telemetry_provider_missing_file_format() {
        let configuration_registry = ConfigurationProviderRegistry::default();
        let err =
            TelemetryProviders::configure_from_yaml_str(&configuration_registry, "").unwrap_err();
        assert!(err
            .to_string()
            .contains("Missing required field `file_format`"));
    }
}
