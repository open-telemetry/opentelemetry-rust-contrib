//! # Provider objects for OpenTelemetry SDKs
//!
//! This module provides the different element providers to configure
//! OpenTelemetry SDKs using declarative YAML configurations.

mod meter_provider;

use std::collections::HashMap;

use opentelemetry::KeyValue;
use opentelemetry_sdk::{
    logs::SdkLoggerProvider, metrics::SdkMeterProvider, trace::SdkTracerProvider, Resource,
};

use crate::{
    model::Telemetry, providers::meter_provider::MeterProvider, ConfigurationProviderRegistry,
    ProviderError,
};

/// Holds the configured telemetry providers
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
    fn configure(
        configuration_registry: &ConfigurationProviderRegistry,
        config: &Telemetry,
    ) -> Result<TelemetryProviders, ProviderError> {
        let resource: Resource = Self::as_resource(&config.resource);

        let meter_provider_option: Option<SdkMeterProvider>;
        if let Some(metrics_config) = &config.metrics {
            let mut meter_provider_builder =
                SdkMeterProvider::builder().with_resource(resource.clone());
            meter_provider_builder = MeterProvider::configure(
                &configuration_registry.metrics,
                meter_provider_builder,
                metrics_config,
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
        let config: crate::model::Telemetry = serde_yaml::from_str(yaml_str).map_err(|e| {
            ProviderError::InvalidConfiguration(format!(
                "Failed to parse YAML configuration: {}",
                e
            ))
        })?;
        Self::configure(configuration_registry, &config)
    }

    /// Converts resource attributes from HashMap to Resource
    fn as_resource(attributes: &HashMap<String, serde_yaml::Value>) -> Resource {
        let mut builder = Resource::builder();

        for (key, value) in attributes {
            let resource_attribute = Self::as_resource_attribute(key, value);
            builder = builder.with_attribute(resource_attribute);
        }

        builder.build()
    }

    /// Converts a single resource attribute from serde_yaml::Value to KeyValue
    fn as_resource_attribute(key: &str, value: &serde_yaml::Value) -> KeyValue {
        // TODO: Add support for arrays.
        match value {
            serde_yaml::Value::String(s) => KeyValue::new(key.to_string(), s.clone()),
            serde_yaml::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    KeyValue::new(key.to_string(), i)
                } else if let Some(f) = n.as_f64() {
                    KeyValue::new(key.to_string(), f)
                } else {
                    KeyValue::new(key.to_string(), n.to_string())
                }
            }
            serde_yaml::Value::Bool(b) => KeyValue::new(key.to_string(), *b),
            _ => KeyValue::new(key.to_string(), format!("{:?}", value)),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ConfigurationError;
    use opentelemetry_sdk::{
        error::OTelSdkResult,
        metrics::{
            data::ResourceMetrics, exporter::PushMetricExporter, MeterProviderBuilder, Temporality,
        },
    };

    use super::*;

    #[derive(Default)]
    struct MockExporter {}

    impl PushMetricExporter for MockExporter {
        async fn export(&self, _metrics: &ResourceMetrics) -> OTelSdkResult {
            Ok(())
        }

        fn force_flush(&self) -> OTelSdkResult {
            Ok(())
        }

        fn shutdown_with_timeout(&self, _timeout: std::time::Duration) -> OTelSdkResult {
            Ok(())
        }

        fn temporality(&self) -> Temporality {
            Temporality::Delta
        }

        fn shutdown(&self) -> OTelSdkResult {
            self.shutdown_with_timeout(std::time::Duration::from_secs(5))
        }
    }

    pub fn register_mock_reader_factory(
        mut builder: MeterProviderBuilder,
        _config_yaml: &str,
    ) -> Result<MeterProviderBuilder, ConfigurationError> {
        let exporter = MockExporter::default();
        builder = builder.with_periodic_exporter(exporter);
        Ok(builder)
    }

    #[test]
    fn test_configure_telemetry_from_yaml() {
        let yaml_str = r#"
        metrics:    
          readers:
            - periodic:
                exporter:
                    console:
                        temporality: delta
        resource:
          service.name: "test-service"
          service.version: "1.0.0"
          replica.count: 3
          cores: 4.5
          development: true
        "#;

        let mut registry = ConfigurationProviderRegistry::default();
        let name = "console";
        registry.register_metric_exporter_factory(name, register_mock_reader_factory);

        let providers = TelemetryProviders::configure_from_yaml_str(&registry, yaml_str).unwrap();
        assert!(providers.meter_provider.is_some());
    }

    #[test]
    fn test_telemetry_provider_default() {
        let configuration_registry = ConfigurationProviderRegistry::default();
        let telemetry = Telemetry {
            resource: HashMap::new(),
            metrics: None,
        };
        let providers = TelemetryProviders::configure(&configuration_registry, &telemetry).unwrap();
        assert!(providers.meter_provider.is_none());
    }

    #[test]
    fn test_telemetry_provider_default_empty_yaml() {
        let configuration_registry = ConfigurationProviderRegistry::default();
        let telemetry: Telemetry = serde_yaml::from_str("").unwrap();
        let providers = TelemetryProviders::configure(&configuration_registry, &telemetry).unwrap();
        assert!(providers.meter_provider.is_none());
    }
}
