//! # Provider objects for OpenTelemetry SDKs
//!
//! This module provides the different element providers to configure
//! OpenTelemetry SDKs using declarative YAML configurations.

pub mod metrics_provider;

use std::collections::HashMap;

use opentelemetry::KeyValue;
use opentelemetry_sdk::{metrics::SdkMeterProvider, Resource};

use crate::{
    model::Telemetry, providers::metrics_provider::MetricsProvider, ConfigurationProvidersRegistry,
    ProviderError, TelemetryProviders,
};

/// Provider for Telemetry object
pub struct TelemetryProvider {
    metrics_provider: MetricsProvider,
}

impl TelemetryProvider {
    /// Creates a new TelemetryProvider
    pub fn new() -> Self {
        Self {
            metrics_provider: MetricsProvider::new(),
        }
    }

    /// Configures the Telemetry providers based on the provided configuration
    pub fn configure(
        &self,
        configuration_registry: &ConfigurationProvidersRegistry,
        config: &Telemetry,
    ) -> Result<TelemetryProviders, ProviderError> {
        let mut providers = TelemetryProviders::new();
        let resource: Resource = self.as_resource(&config.resource);
        if let Some(metrics_config) = &config.metrics {
            let mut meter_provider_builder =
                SdkMeterProvider::builder().with_resource(resource.clone());
            meter_provider_builder = self.metrics_provider.configure(
                configuration_registry.metrics(),
                meter_provider_builder,
                metrics_config,
            )?;
            let meter_provider = meter_provider_builder.build();
            providers = providers.with_meter_provider(meter_provider);
        }

        // TODO: Add traces and logs configuration

        Ok(providers)
    }

    /// Configures the Telemetry providers from a YAML string
    pub fn configure_from_yaml(
        &self,
        configuration_registry: &ConfigurationProvidersRegistry,
        yaml_str: &str,
    ) -> Result<TelemetryProviders, ProviderError> {
        let config: crate::model::Telemetry = serde_yaml::from_str(yaml_str).map_err(|e| {
            ProviderError::InvalidConfiguration(format!(
                "Failed to parse YAML configuration: {}",
                e
            ))
        })?;
        self.configure(configuration_registry, &config)
    }

    /// Configures the Telemetry providers from a YAML file
    pub fn configure_from_yaml_file(
        &self,
        configuration_registry: &ConfigurationProvidersRegistry,
        file_path: &str,
    ) -> Result<TelemetryProviders, ProviderError> {
        let yaml_str = std::fs::read_to_string(file_path).map_err(|e| {
            ProviderError::InvalidConfiguration(format!(
                "Failed to read YAML configuration file: {}",
                e
            ))
        })?;
        self.configure_from_yaml(configuration_registry, &yaml_str)
    }

    /// Converts resource attributes from HashMap to Resource
    fn as_resource(&self, attributes: &HashMap<String, serde_yaml::Value>) -> Resource {
        let mut builder = Resource::builder();

        for (key, value) in attributes {
            let resource_attribute = self.as_resource_attribute(key, value);
            builder = builder.with_attribute(resource_attribute);
        }

        builder.build()
    }

    /// Converts a single resource attribute from serde_yaml::Value to KeyValue
    fn as_resource_attribute(&self, key: &str, value: &serde_yaml::Value) -> KeyValue {
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

impl Default for TelemetryProvider {
    fn default() -> Self {
        Self::new()
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

    struct MockExporter {}
    impl MockExporter {
        fn new() -> Self {
            Self {}
        }
    }

    impl PushMetricExporter for MockExporter {
        fn export(
            &self,
            _metrics: &ResourceMetrics,
        ) -> impl std::future::Future<Output = OTelSdkResult> + Send {
            async move { Ok(()) }
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

    pub fn register_mock_exporter(
        mut builder: MeterProviderBuilder,
        _config: &serde_yaml::Value,
    ) -> Result<MeterProviderBuilder, ConfigurationError> {
        let exporter = MockExporter::new();
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

        let mut configuration_registry = ConfigurationProvidersRegistry::new();
        let metrics_provider_manager = configuration_registry.metrics_mut();
        let name = "console";
        metrics_provider_manager
            .register_periodic_exporter_factory(name.to_string(), register_mock_exporter);

        let telemetry_provider = TelemetryProvider::new();
        let providers = telemetry_provider
            .configure_from_yaml(&configuration_registry, yaml_str)
            .unwrap();
        assert!(providers.meter_provider.is_some());
    }

    #[test]
    fn test_telemetry_provider_default() {
        let telemetry_provider = TelemetryProvider::default();
        let configuration_registry = ConfigurationProvidersRegistry::default();
        let telemetry = Telemetry {
            resource: HashMap::new(),
            metrics: None,
        };
        let providers = telemetry_provider
            .configure(&configuration_registry, &telemetry)
            .unwrap();
        assert!(providers.meter_provider.is_none());
    }

    #[test]
    fn test_telemetry_provider_default_empty_yaml() {
        let telemetry_provider = TelemetryProvider::default();
        let configuration_registry = ConfigurationProvidersRegistry::default();
        let telemetry: Telemetry = serde_yaml::from_str("").unwrap();
        let providers = telemetry_provider
            .configure(&configuration_registry, &telemetry)
            .unwrap();
        assert!(providers.meter_provider.is_none());
    }
}
