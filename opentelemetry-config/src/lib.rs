//! # Library for declarative configuration of OpenTelemetry.
//!
//! This library provides a way to configure OpenTelemetry SDK components
//! using a declarative approach. It allows users to define configurations
//! for metrics, traces, and exporters in a structured manner.

use std::{
    collections::HashMap,
    error::{self, Error},
    fmt::{self, Display},
};

use opentelemetry_sdk::metrics::MeterProviderBuilder;

pub mod model;
pub mod providers;

/// Registry for different configuration providers.
#[derive(Default)]
pub struct ConfigurationProviderRegistry {
    metrics: MeterProviderRegistry,
    // TODO: Add logs and traces providers registries.
}

impl ConfigurationProviderRegistry {
    /// Registers a new MeterProvider factory with the given name.
    /// The factory is a function that takes a MeterProviderBuilder and a YAML configuration string,
    /// and returns a configured MeterProviderBuilder or a ConfigurationError.
    pub fn register_metric_exporter_factory(
        &mut self,
        name: &str,
        factory: impl Fn(MeterProviderBuilder, &str) -> Result<MeterProviderBuilder, ConfigurationError>
            + 'static,
    ) {
        self.metrics.register_exporter_factory(name, factory);
    }
}

/// Registry for metrics configuration providers.
#[derive(Default)]
pub(crate) struct MeterProviderRegistry {
    provider_factories: HashMap<String, Box<MeterProviderFactory>>,
    // TODO: Add other types of providers registries.
}

impl MeterProviderRegistry {
    /// Registers a new exporter factory with the given name.
    /// The factory is a function that takes a MeterProviderBuilder and a YAML configuration string,
    /// and returns a configured MeterProviderBuilder or a ConfigurationError.
    pub(crate) fn register_exporter_factory(
        &mut self,
        name: &str,
        factory: impl Fn(MeterProviderBuilder, &str) -> Result<MeterProviderBuilder, ConfigurationError>
            + 'static,
    ) {
        self.provider_factories
            .insert(name.to_string(), Box::new(factory));
    }

    /// Retrieves a provider factory by name.
    pub(crate) fn provider_factory(&self, name: &str) -> Option<&MeterProviderFactory> {
        self.provider_factories
            .get(name)
            .map(|boxed_factory| boxed_factory.as_ref())
    }
}

/// Errors reported by the component factory.
#[derive(Debug)]
pub enum ConfigurationError {
    /// Indicates an invalid configuration was provided.
    InvalidConfiguration(String),

    /// Indicates an error occurred while registering a component.
    RegistrationError(String),
}

impl Error for ConfigurationError {}

impl fmt::Display for ConfigurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigurationError::InvalidConfiguration(details) => {
                write!(f, "Invalid configuration: {}", details)
            }
            ConfigurationError::RegistrationError(details) => {
                write!(f, "Registration error: {}", details)
            }
        }
    }
}

/// Type alias for meter provider factory functions
/// that create meter providers based on a given yaml configuration string.
type MeterProviderFactory =
    dyn Fn(MeterProviderBuilder, &str) -> Result<MeterProviderBuilder, ConfigurationError>;

/// Errors related to providers and configuration management.
#[derive(Debug)]
pub enum ProviderError {
    InvalidConfiguration(String),
    UnsupportedExporter(String),
    NotRegisteredProvider(String),
    RegistrationError(String),
}

impl error::Error for ProviderError {}

impl Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderError::InvalidConfiguration(details) => {
                write!(f, "Invalid configuration: {}", details)
            }
            ProviderError::UnsupportedExporter(details) => {
                write!(f, "Unsupported exporter: {}", details)
            }
            ProviderError::NotRegisteredProvider(details) => {
                write!(f, "Not registered provider: {}", details)
            }
            ProviderError::RegistrationError(details) => {
                write!(f, "Registration error: {}", details)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use opentelemetry_sdk::{
        error::OTelSdkResult,
        metrics::{data::ResourceMetrics, exporter::PushMetricExporter, PeriodicReader},
    };

    use serde_yaml::Value;

    use super::*;

    #[test]
    fn test_register_periodic_reader_factory() {
        // Arrange
        #[derive(serde::Deserialize, Debug)]
        pub struct MockPeriodicExporter {
            pub console: Option<MockConsoleConfig>,
        }

        #[derive(serde::Deserialize, Debug)]
        pub struct MockConsoleConfig {
            pub temporality: Option<String>,
        }

        impl PushMetricExporter for MockPeriodicExporter {
            async fn export(&self, _metrics: &ResourceMetrics) -> OTelSdkResult {
                Ok(())
            }

            fn force_flush(&self) -> OTelSdkResult {
                Ok(())
            }

            fn shutdown_with_timeout(&self, _timeout: std::time::Duration) -> OTelSdkResult {
                Ok(())
            }

            fn shutdown(&self) -> OTelSdkResult {
                Ok(())
            }

            fn temporality(&self) -> opentelemetry_sdk::metrics::Temporality {
                opentelemetry_sdk::metrics::Temporality::Cumulative
            }
        }

        let call_count = Rc::new(Cell::new(0));
        let call_count_clone = Rc::clone(&call_count);

        // Wrapper clousure to capture call_count_clone
        let register_mock_reader_clousure =
            move |builder: MeterProviderBuilder, periodic_config: &str| {
                call_count_clone.set(call_count_clone.get() + 1);
                register_mock_reader(builder, periodic_config)
            };

        pub fn register_mock_reader(
            mut builder: MeterProviderBuilder,
            config_yaml_str: &str,
        ) -> Result<MeterProviderBuilder, ConfigurationError> {
            let config: Value = serde_yaml::from_str(config_yaml_str).map_err(|e| {
                ConfigurationError::InvalidConfiguration(format!(
                    "Failed to parse configuration YAML: {}",
                    e
                ))
            })?;
            let exporter: MockPeriodicExporter = serde_yaml::from_value(config["exporter"].clone())
                .map_err(|e| {
                    ConfigurationError::InvalidConfiguration(format!(
                        "Failed to parse MockPeriodicExporter: {}",
                        e
                    ))
                })?;
            assert!(exporter
                .console
                .as_ref()
                .and_then(|c| c.temporality.as_ref())
                .is_some());
            let interval_millis = config["interval"].as_u64().unwrap_or(60000);
            let reader = PeriodicReader::builder(exporter)
                .with_interval(std::time::Duration::from_millis(interval_millis))
                .build();
            builder = builder.with_reader(reader);
            Ok(builder)
        }

        let mut registry = ConfigurationProviderRegistry::default();

        // Act
        let name = "console";
        registry.register_metric_exporter_factory(name, register_mock_reader_clousure);

        // Assert
        assert!(registry.metrics.provider_factories.contains_key(name));
        let periodic_config_yaml = r#"
            interval: 1000
            timeout: 5000
            exporter:
              console:
                temporality: cumulative
            "#;

        let factory_function_option = registry.metrics.provider_factory(name);
        if let Some(factory_function) = factory_function_option {
            let builder = MeterProviderBuilder::default();
            _ = factory_function(builder, periodic_config_yaml).unwrap();
            // Verify that the factory function was called
            assert_eq!(call_count.get(), 1);
        } else {
            panic!("Provider not found");
        }
    }

    #[test]
    fn test_provider_manager_default() {
        let provider_manager = ConfigurationProviderRegistry::default();
        assert!(provider_manager.metrics.provider_factories.is_empty());
    }

    #[test]
    fn test_metrics_provider_registry_default() {
        let metrics_provider_registry = MeterProviderRegistry::default();
        assert!(metrics_provider_registry.provider_factories.is_empty());
    }

    #[test]
    fn test_has_periodic_reader_factory() {
        let mut registry = MeterProviderRegistry::default();
        let name = "test_factory";
        assert!(!registry.provider_factories.contains_key(name));
        registry.register_exporter_factory(name, |builder, _| Ok(builder));
        assert!(registry.provider_factories.contains_key(name));
    }
}
