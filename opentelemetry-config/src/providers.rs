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

    pub fn provide(
        &self,
        configuration_registry: &ConfigurationProvidersRegistry,
        config: &Telemetry,
    ) -> Result<TelemetryProviders, ProviderError> {
        let mut providers = TelemetryProviders::new();
        let resource: Resource = self.as_resource(&config.resource);
        if let Some(metrics_config) = &config.metrics {
            let mut meter_provider_builder =
                SdkMeterProvider::builder().with_resource(resource.clone());
            meter_provider_builder = self.metrics_provider.provide(
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

    pub fn provide_from_yaml(
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
        self.provide(configuration_registry, &config)
    }

    pub fn provide_from_yaml_file(
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
        self.provide_from_yaml(configuration_registry, &yaml_str)
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
    use std::{any::Any, cell::Cell};

    use opentelemetry_sdk::metrics::MeterProviderBuilder;

    use crate::{
        model::metrics::reader::PeriodicExporterConsole, MetricsReaderPeriodicExporterProvider,
    };

    use super::*;

    struct MockMetricsReadersPeriodicExporterConsoleProvider {
        call_count: Cell<u16>,
    }

    impl MockMetricsReadersPeriodicExporterConsoleProvider {
        fn new() -> Self {
            Self {
                call_count: Cell::new(0),
            }
        }

        pub fn get_call_count(&self) -> u16 {
            self.call_count.get()
        }
    }

    impl MetricsReaderPeriodicExporterProvider for MockMetricsReadersPeriodicExporterConsoleProvider {
        fn provide(
            &self,
            meter_provider_builder: MeterProviderBuilder,
            config: &(dyn Any + 'static),
        ) -> MeterProviderBuilder {
            // Mock implementation: In a real scenario, configure the console exporter here
            self.call_count.set(self.call_count.get() + 1);
            let config = config
                .downcast_ref::<PeriodicExporterConsole>()
                .expect("Invalid config type");
            println!("Mock configure called with config: {:?}", config);
            meter_provider_builder
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
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

        let provider = Box::new(MockMetricsReadersPeriodicExporterConsoleProvider::new());

        let mut configuration_registry = ConfigurationProvidersRegistry::new();
        let metrics_provider_manager = configuration_registry.metrics_mut();
        //metrics_provider_manager.register_periodic_exporter_console_provider(provider);
        metrics_provider_manager
            .register_periodic_exporter_provider::<PeriodicExporterConsole>(provider);

        let telemetry_provider = TelemetryProvider::new();
        let providers = telemetry_provider
            .provide_from_yaml(&configuration_registry, yaml_str)
            .unwrap();
        assert!(providers.meter_provider.is_some());

        let provider = configuration_registry
            .metrics()
            .readers_periodic_exporter::<PeriodicExporterConsole>()
            .unwrap();
        let provider = provider
            .as_any()
            .downcast_ref::<MockMetricsReadersPeriodicExporterConsoleProvider>()
            .unwrap();
        assert_eq!(provider.get_call_count(), 1);
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
            .provide(&configuration_registry, &telemetry)
            .unwrap();
        assert!(providers.meter_provider.is_none());
    }
}
