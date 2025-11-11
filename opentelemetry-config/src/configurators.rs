//! # Configurator objects for OpenTelemetry SDKs
//!
//! This module provides the different element configurators to configure
//! OpenTelemetry SDKs using declarative YAML configurations.

pub mod metrics_configurator;

use std::collections::HashMap;

use opentelemetry::KeyValue;
use opentelemetry_sdk::{metrics::SdkMeterProvider, Resource};

use crate::{
    configurators::metrics_configurator::MetricsConfigurator, model::Telemetry, ConfiguratorError,
    ConfiguratorManager, TelemetryProviders,
};

/// Configurator for Telemetry object
pub struct TelemetryConfigurator {
    metrics_configurator: MetricsConfigurator,
}

impl TelemetryConfigurator {
    /// Creates a new TelemetryConfigurator
    pub fn new() -> Self {
        Self {
            metrics_configurator: MetricsConfigurator::new(),
        }
    }

    pub fn configure(
        &self,
        configurator_manager: &ConfiguratorManager,
        config: &Telemetry,
    ) -> Result<TelemetryProviders, ConfiguratorError> {
        let mut providers = TelemetryProviders::new();
        let resource: Resource = self.as_resource(&config.resource);
        if let Some(metrics_config) = &config.metrics {
            let mut meter_provider_builder =
                SdkMeterProvider::builder().with_resource(resource.clone());
            meter_provider_builder = self.metrics_configurator.configure(
                configurator_manager.metrics(),
                meter_provider_builder,
                metrics_config,
            )?;
            let meter_provider = meter_provider_builder.build();
            providers = providers.with_meter_provider(meter_provider);
        }

        // TODO: Add traces and logs configuration

        Ok(providers)
    }

    pub fn configure_from_yaml(
        &self,
        configurator_manager: &ConfiguratorManager,
        yaml_str: &str,
    ) -> Result<TelemetryProviders, ConfiguratorError> {
        let config: crate::model::Telemetry = serde_yaml::from_str(yaml_str).map_err(|e| {
            ConfiguratorError::InvalidConfiguration(format!(
                "Failed to parse YAML configuration: {}",
                e
            ))
        })?;
        self.configure(configurator_manager, &config)
    }

    pub fn configure_from_yaml_file(
        &self,
        configurator_manager: &ConfiguratorManager,
        file_path: &str,
    ) -> Result<TelemetryProviders, ConfiguratorError> {
        let yaml_str = std::fs::read_to_string(file_path).map_err(|e| {
            ConfiguratorError::InvalidConfiguration(format!(
                "Failed to read YAML configuration file: {}",
                e
            ))
        })?;
        self.configure_from_yaml(configurator_manager, &yaml_str)
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

impl Default for TelemetryConfigurator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::{any::Any, sync::atomic::AtomicU16};

    use opentelemetry_sdk::metrics::MeterProviderBuilder;

    use crate::{
        model::metrics::reader::PeriodicExporterConsole, MetricsReaderPeriodicExporterConfigurator,
    };

    use super::*;

    struct MockMetricsReadersPeriodicExporterConsoleConfigurator {
        call_count: AtomicU16,
    }

    impl MockMetricsReadersPeriodicExporterConsoleConfigurator {
        fn new() -> Self {
            Self {
                call_count: AtomicU16::new(0),
            }
        }

        pub fn get_call_count(&self) -> u16 {
            self.call_count.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl MetricsReaderPeriodicExporterConfigurator
        for MockMetricsReadersPeriodicExporterConsoleConfigurator
    {
        fn configure(
            &self,
            meter_provider_builder: MeterProviderBuilder,
            config: &(dyn Any + 'static),
        ) -> MeterProviderBuilder {
            // Mock implementation: In a real scenario, configure the console exporter here
            self.call_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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

        let configurator = Box::new(MockMetricsReadersPeriodicExporterConsoleConfigurator::new());

        let mut configurator_manager = ConfiguratorManager::new();
        let metrics_configurator_manager = configurator_manager.metrics_mut();
        //metrics_configurator_manager.register_periodic_exporter_console_configurator(configurator);
        metrics_configurator_manager
            .register_periodic_exporter_configurator::<PeriodicExporterConsole>(configurator);

        let telemetry_configurator = TelemetryConfigurator::new();
        let providers = telemetry_configurator
            .configure_from_yaml(&configurator_manager, yaml_str)
            .unwrap();
        assert!(providers.meter_provider.is_some());

        let configurator = configurator_manager
            .metrics()
            .readers_periodic_exporter::<PeriodicExporterConsole>()
            .unwrap();
        let configurator = configurator
            .as_any()
            .downcast_ref::<MockMetricsReadersPeriodicExporterConsoleConfigurator>()
            .unwrap();
        assert_eq!(configurator.get_call_count(), 1);
    }

    #[test]
    fn test_telemetry_configurator_default() {
        let telemetry_configurator = TelemetryConfigurator::default();
        let configurator_manager = ConfiguratorManager::default();
        let telemetry = Telemetry {
            resource: HashMap::new(),
            metrics: None,
        };
        let providers = telemetry_configurator
            .configure(&configurator_manager, &telemetry)
            .unwrap();
        assert!(providers.meter_provider.is_none());
    }
}
