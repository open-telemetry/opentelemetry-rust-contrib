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
    error::OTelSdkResult,
    logs::SdkLoggerProvider,
    metrics::{MeterProviderBuilder, SdkMeterProvider},
    trace::SdkTracerProvider,
};

use crate::model::metrics::reader::{PeriodicExporterConsole, PeriodicExporterOtlp};

pub mod configurators;
pub mod model;

pub struct ConfiguratorManager {
    metrics: MetricsConfiguratorManager,
}

impl ConfiguratorManager {
    pub fn new() -> Self {
        Self {
            metrics: MetricsConfiguratorManager::new(),
        }
    }

    pub fn metrics_mut(&mut self) -> &mut MetricsConfiguratorManager {
        &mut self.metrics
    }

    pub fn metrics(&self) -> &MetricsConfiguratorManager {
        &self.metrics
    }
}

impl Default for ConfiguratorManager {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MetricsConfiguratorManager {
    readers_periodic_exporters: HashMap<String, Box<dyn MetricsReaderPeriodicExporterConfigurator>>,
}

impl MetricsConfiguratorManager {
    pub fn new() -> Self {
        Self {
            readers_periodic_exporters: HashMap::new(),
        }
    }

    pub fn register_periodic_exporter_configurator<T: 'static + std::any::Any + ?Sized>(
        &mut self,
        configurator: Box<dyn MetricsReaderPeriodicExporterConfigurator>,
    ) {
        let name: String = type_name::<T>().to_string();
        self.readers_periodic_exporters.insert(
            name,
            configurator as Box<dyn MetricsReaderPeriodicExporterConfigurator>,
        );
    }

    pub fn readers_periodic_exporter<T>(
        &self,
        //type_name: &str,
    ) -> Option<&dyn MetricsReaderPeriodicExporterConfigurator> {
        let type_name = type_name::<T>().to_string();
        self.readers_periodic_exporters
            .get(&type_name)
            .map(|b| b.as_ref())
    }
}

impl Default for MetricsConfiguratorManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for configuring Console Exporter for Periodic Metrics Reader
pub trait MetricsReadersPeriodicExporterConsoleConfigurator {
    fn configure(
        &self,
        meter_provider_builder: MeterProviderBuilder,
        config: &PeriodicExporterConsole,
    ) -> MeterProviderBuilder;

    fn as_any(&self) -> &dyn std::any::Any;
}

/// Trait for configuring OTLP Exporter for Periodic Metrics Reader
pub trait MetricsReadersPeriodicExporterOtlpConfigurator {
    fn configure(
        &self,
        meter_provider_builder: MeterProviderBuilder,
        config: &PeriodicExporterOtlp,
    ) -> MeterProviderBuilder;
}

pub trait MetricsReaderPeriodicExporterConfigurator {
    fn configure(
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

    pub fn shutdown(self) -> OTelSdkResult {
        if let Some(meter_provider) = self.meter_provider {
            meter_provider.shutdown()?;
        }
        if let Some(traces_provider) = self.traces_provider {
            traces_provider.shutdown()?;
        }
        if let Some(logs_provider) = self.logs_provider {
            logs_provider.shutdown()?;
        }
        Ok(())
    }
}

impl Default for TelemetryProviders {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum ConfiguratorError {
    InvalidConfiguration(String),
    UnsupportedExporter(String),
    NotRegisteredConfigurator(String),
}

impl error::Error for ConfiguratorError {}

impl Display for ConfiguratorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfiguratorError::InvalidConfiguration(details) => {
                write!(f, "Invalid configuration: {}", details)
            }
            ConfiguratorError::UnsupportedExporter(details) => {
                write!(f, "Unsupported exporter: {}", details)
            }
            ConfiguratorError::NotRegisteredConfigurator(details) => {
                write!(f, "Not registered configurator: {}", details)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicI16;

    use super::*;

    #[test]
    fn test_register_periodic_exporter_configurator() {
        // Arrange
        struct MockPeriodicExporterConfigurator {
            call_count: AtomicI16,
        }

        impl MockPeriodicExporterConfigurator {
            fn new() -> Self {
                Self {
                    call_count: AtomicI16::new(0),
                }
            }

            pub fn get_call_count(&self) -> i16 {
                self.call_count.load(std::sync::atomic::Ordering::SeqCst)
            }
        }

        impl MetricsReaderPeriodicExporterConfigurator for MockPeriodicExporterConfigurator {
            fn configure(
                &self,
                meter_provider_builder: MeterProviderBuilder,
                _config: &dyn std::any::Any,
            ) -> MeterProviderBuilder {
                self.call_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                meter_provider_builder
            }

            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }

        let mock_configurator = Box::new(MockPeriodicExporterConfigurator::new());
        let mut configurator_manager = ConfiguratorManager::new();

        // Act
        configurator_manager
            .metrics_mut()
            .register_periodic_exporter_configurator::<PeriodicExporterConsole>(mock_configurator);

        // Assert
        let type_name = type_name::<PeriodicExporterConsole>().to_string();
        assert!(configurator_manager
            .metrics()
            .readers_periodic_exporters
            .contains_key(&type_name));

        let configurator_option = configurator_manager
            .metrics()
            .readers_periodic_exporter::<PeriodicExporterConsole>();
        if let Some(configurator) = configurator_option {
            configurator.configure(
                MeterProviderBuilder::default(),
                &PeriodicExporterConsole { temporality: None },
            );
            let configurator_cast = configurator
                .as_any()
                .downcast_ref::<MockPeriodicExporterConfigurator>()
                .unwrap();
            assert_eq!(configurator_cast.get_call_count(), 1);
        } else {
            panic!("Configurator not found");
        }
    }

    #[test]
    fn test_configurator_manager_default() {
        let configurator_manager = ConfiguratorManager::default();
        assert!(configurator_manager
            .metrics()
            .readers_periodic_exporters
            .is_empty());
    }

    #[test]
    fn test_metrics_configurator_manager_default() {
        let metrics_configurator_manager = MetricsConfiguratorManager::default();
        assert!(metrics_configurator_manager
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
