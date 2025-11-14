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

use opentelemetry_sdk::{
    logs::SdkLoggerProvider,
    metrics::{MeterProviderBuilder, SdkMeterProvider},
    trace::SdkTracerProvider,
};
use serde_yaml::Value;

pub mod model;
pub mod providers;

/// Registry for different configuration providers.
pub struct ConfigurationProvidersRegistry {
    metrics: MetricsProvidersRegistry,
}

impl ConfigurationProvidersRegistry {
    pub fn new() -> Self {
        Self {
            metrics: MetricsProvidersRegistry::new(),
        }
    }

    pub fn metrics_mut(&mut self) -> &mut MetricsProvidersRegistry {
        &mut self.metrics
    }

    pub fn metrics(&self) -> &MetricsProvidersRegistry {
        &self.metrics
    }
}

impl Default for ConfigurationProvidersRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Registry for metrics configuration providers.
pub struct MetricsProvidersRegistry {
    periodic_exporter_factories: HashMap<String, Box<MetricConfigFactory>>,
    // TODO: Add other types of providers registries.
}

impl MetricsProvidersRegistry {
    pub fn new() -> Self {
        Self {
            periodic_exporter_factories: HashMap::new(),
        }
    }

    pub fn register_periodic_exporter_factory(
        &mut self,
        name: String,
        factory: impl Fn(MeterProviderBuilder, &Value) -> Result<MeterProviderBuilder, ConfigurationError>
            + Send
            + Sync
            + 'static,
    ) {
        self.periodic_exporter_factories
            .insert(name, Box::new(factory));
    }

    pub fn periodic_exporter_factory(&self, name: &str) -> Option<&Box<MetricConfigFactory>> {
        self.periodic_exporter_factories.get(name)
    }
}

impl Default for MetricsProvidersRegistry {
    fn default() -> Self {
        Self::new()
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

/// Type alias for metric configuration factory functions
pub type MetricConfigFactory = dyn Fn(MeterProviderBuilder, &Value) -> Result<MeterProviderBuilder, ConfigurationError>
    + Send
    + Sync;

/// Holds the configured telemetry providers
pub struct TelemetryProviders {
    meter_provider: Option<SdkMeterProvider>,
    traces_provider: Option<SdkTracerProvider>,
    logs_provider: Option<SdkLoggerProvider>,
}

impl TelemetryProviders {
    pub fn new() -> Self {
        TelemetryProviders {
            meter_provider: None,
            traces_provider: None,
            logs_provider: None,
        }
    }

    pub fn with_meter_provider(mut self, meter_provider: SdkMeterProvider) -> Self {
        self.meter_provider = Some(meter_provider);
        self
    }

    pub fn with_traces_provider(mut self, traces_provider: SdkTracerProvider) -> Self {
        self.traces_provider = Some(traces_provider);
        self
    }
    pub fn with_logs_provider(mut self, logs_provider: SdkLoggerProvider) -> Self {
        self.logs_provider = Some(logs_provider);
        self
    }

    pub fn meter_provider(&self) -> Option<&SdkMeterProvider> {
        self.meter_provider.as_ref()
    }

    pub fn traces_provider(&self) -> Option<&SdkTracerProvider> {
        self.traces_provider.as_ref()
    }

    pub fn logs_provider(&self) -> Option<&SdkLoggerProvider> {
        self.logs_provider.as_ref()
    }
}

/// Default implementation for TelemetryProviders
impl Default for TelemetryProviders {
    fn default() -> Self {
        Self::new()
    }
}

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
    use std::sync::{atomic::AtomicI16, Arc};

    use opentelemetry_sdk::{
        error::OTelSdkResult,
        metrics::{data::ResourceMetrics, exporter::PushMetricExporter},
    };

    use super::*;

    #[test]
    fn test_register_periodic_exporter_provider() {
        // Arrange
        struct MockPeriodicExporter {}

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

        let call_count = Arc::new(AtomicI16::new(0));
        let call_count_clone = Arc::clone(&call_count);

        // Wrapper clousure to capture call_count_clone
        let register_mock_exporter_clousure =
            move |builder: MeterProviderBuilder, _config: &serde_yaml::Value| {
                call_count_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                register_mock_exporter(builder, _config)
            };

        pub fn register_mock_exporter(
            mut builder: MeterProviderBuilder,
            _config: &serde_yaml::Value,
        ) -> Result<MeterProviderBuilder, ConfigurationError> {
            builder = builder.with_periodic_exporter(MockPeriodicExporter {});
            Ok(builder)
        }

        let mut registry = ConfigurationProvidersRegistry::new();

        // Act
        let name = "console";
        registry
            .metrics_mut()
            .register_periodic_exporter_factory(name.to_string(), register_mock_exporter_clousure);

        // Assert
        assert!(registry
            .metrics()
            .periodic_exporter_factories
            .contains_key(name));

        let console_config = serde_yaml::to_value(
            r#"
            console:
              temporality: cumulative
            "#,
        )
        .unwrap();

        let factory_function_option = registry.metrics().periodic_exporter_factory(&name);
        if let Some(factory_function) = factory_function_option {
            let builder = MeterProviderBuilder::default();
            _ = factory_function(builder, &console_config).unwrap();
            // Verify that the factory function was called
            assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        } else {
            panic!("Provider not found");
        }
    }

    #[test]
    fn test_provider_manager_default() {
        let provider_manager = ConfigurationProvidersRegistry::default();
        assert!(provider_manager
            .metrics()
            .periodic_exporter_factories
            .is_empty());
    }

    #[test]
    fn test_metrics_provider_registry_default() {
        let metrics_provider_registry = MetricsProvidersRegistry::default();
        assert!(metrics_provider_registry
            .periodic_exporter_factories
            .is_empty());
    }

    #[test]
    fn test_telemetry_providers_default() {
        let telemetry_providers = TelemetryProviders::default();
        assert!(telemetry_providers.meter_provider.is_none());
        assert!(telemetry_providers.traces_provider.is_none());
        assert!(telemetry_providers.logs_provider.is_none());
    }

    #[test]
    fn test_telemetry_providers_with_methods() {
        let meter_provider = SdkMeterProvider::builder().build();
        let traces_provider = SdkTracerProvider::builder().build();
        let logs_provider = SdkLoggerProvider::builder().build();

        let telemetry_providers = TelemetryProviders::new()
            .with_logs_provider(logs_provider)
            .with_traces_provider(traces_provider)
            .with_meter_provider(meter_provider);

        assert!(telemetry_providers.meter_provider().is_some());
        assert!(telemetry_providers.traces_provider().is_some());
        assert!(telemetry_providers.logs_provider().is_some());
    }
}
