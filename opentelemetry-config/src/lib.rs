//! # Library for declarative configuration of OpenTelemetry.
//!
//! This library provides a way to configure OpenTelemetry SDK components
//! using a declarative approach. It allows users to define configurations
//! for metrics, traces, and exporters in a structured manner.

use std::{
    any::type_name,
    collections::HashMap,
    error,
    fmt::{self, Display},
};

use opentelemetry_sdk::{
    logs::SdkLoggerProvider,
    metrics::{MeterProviderBuilder, SdkMeterProvider},
    trace::SdkTracerProvider,
};

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
    readers_periodic_exporters: HashMap<String, Box<dyn MetricsReaderPeriodicExporterProvider>>,
}

impl MetricsProvidersRegistry {
    pub fn new() -> Self {
        Self {
            readers_periodic_exporters: HashMap::new(),
        }
    }

    pub fn register_periodic_exporter_provider<T: 'static + std::any::Any + ?Sized>(
        &mut self,
        provider: Box<dyn MetricsReaderPeriodicExporterProvider>,
    ) {
        let name: String = type_name::<T>().to_string();
        self.readers_periodic_exporters.insert(
            name,
            provider as Box<dyn MetricsReaderPeriodicExporterProvider>,
        );
    }

    pub fn readers_periodic_exporter<T>(
        &self,
        //type_name: &str,
    ) -> Option<&dyn MetricsReaderPeriodicExporterProvider> {
        let type_name = type_name::<T>().to_string();
        self.readers_periodic_exporters
            .get(&type_name)
            .map(|b| b.as_ref())
    }
}

impl Default for MetricsProvidersRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for providing metrics reader periodic exporter configurations.
pub trait MetricsReaderPeriodicExporterProvider {
    fn provide(
        &self,
        meter_provider_builder: MeterProviderBuilder,
        config: &dyn std::any::Any,
    ) -> MeterProviderBuilder;

    fn as_any(&self) -> &dyn std::any::Any;
}

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
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use crate::model::metrics::reader::PeriodicExporterConsole;

    use super::*;

    #[test]
    fn test_register_periodic_exporter_provider() {
        // Arrange
        struct MockPeriodicExporterProvider {
            call_count: Cell<i16>,
        }

        impl MockPeriodicExporterProvider {
            fn new() -> Self {
                Self {
                    call_count: Cell::new(0),
                }
            }

            pub fn get_call_count(&self) -> i16 {
                self.call_count.get()
            }
        }

        impl MetricsReaderPeriodicExporterProvider for MockPeriodicExporterProvider {
            fn provide(
                &self,
                meter_provider_builder: MeterProviderBuilder,
                _config: &dyn std::any::Any,
            ) -> MeterProviderBuilder {
                self.call_count.set(self.call_count.get() + 1);
                meter_provider_builder
            }

            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }

        let mock_provider = Box::new(MockPeriodicExporterProvider::new());
        let mut registry = ConfigurationProvidersRegistry::new();

        // Act
        registry
            .metrics_mut()
            .register_periodic_exporter_provider::<PeriodicExporterConsole>(mock_provider);

        // Assert
        let type_name = type_name::<PeriodicExporterConsole>().to_string();
        assert!(registry
            .metrics()
            .readers_periodic_exporters
            .contains_key(&type_name));

        let provider_option = registry
            .metrics()
            .readers_periodic_exporter::<PeriodicExporterConsole>();
        if let Some(provider) = provider_option {
            provider.provide(
                MeterProviderBuilder::default(),
                &PeriodicExporterConsole { temporality: None },
            );
            let provider_cast = provider
                .as_any()
                .downcast_ref::<MockPeriodicExporterProvider>()
                .unwrap();
            assert_eq!(provider_cast.get_call_count(), 1);
        } else {
            panic!("Provider not found");
        }
    }

    #[test]
    fn test_provider_manager_default() {
        let provider_manager = ConfigurationProvidersRegistry::default();
        assert!(provider_manager
            .metrics()
            .readers_periodic_exporters
            .is_empty());
    }

    #[test]
    fn test_metrics_provider_manager_default() {
        let metrics_provider_manager = MetricsProvidersRegistry::default();
        assert!(metrics_provider_manager
            .readers_periodic_exporters
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

        let telemetry_providers = TelemetryProviders {
            meter_provider: Some(meter_provider),
            traces_provider: Some(traces_provider),
            logs_provider: Some(logs_provider),
        };

        assert!(telemetry_providers.meter_provider().is_some());
        assert!(telemetry_providers.traces_provider().is_some());
        assert!(telemetry_providers.logs_provider().is_some());
    }
}
