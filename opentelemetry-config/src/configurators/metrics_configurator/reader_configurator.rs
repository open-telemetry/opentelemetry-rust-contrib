//! # Metrics reader configurator module.
//!
//! This module provides configurators for setting up metrics readers
//! in OpenTelemetry SDKs using declarative YAML configurations.

use opentelemetry_sdk::metrics::MeterProviderBuilder;

use crate::{
    model::metrics::reader::{PeriodicExporterConsole, PeriodicExporterOtlp, Reader},
    ConfiguratorError, MetricsConfiguratorManager,
};

/// Configurator for Metrics readers
pub struct ReaderConfigurator {
    periodic_reader_configurator: PeriodicReaderConfigurator,
}

impl ReaderConfigurator {
    pub fn new() -> Self {
        ReaderConfigurator {
            periodic_reader_configurator: PeriodicReaderConfigurator::new(),
        }
    }
    /// Configures a metrics reader based on the provided configuration
    pub fn configure(
        &self,
        metrics_configurator_manager: &MetricsConfiguratorManager,
        mut meter_provider_builder: MeterProviderBuilder,
        config: &Reader,
    ) -> Result<MeterProviderBuilder, ConfiguratorError> {
        match config {
            crate::model::metrics::reader::Reader::Periodic(periodic_config) => {
                meter_provider_builder = self.periodic_reader_configurator.configure(
                    metrics_configurator_manager,
                    meter_provider_builder,
                    periodic_config,
                )?;
            }
            crate::model::metrics::reader::Reader::Pull(_pull_config) => {
                // TODO: Implement pull reader configuration
            }
        }
        Ok(meter_provider_builder)
    }
}

impl Default for ReaderConfigurator {
    fn default() -> Self {
        Self::new()
    }
}

/// Periodic reader configurator
pub struct PeriodicReaderConfigurator {
    periodic_exporter_configurator: PeriodicExporterConfigurator,
}

impl PeriodicReaderConfigurator {
    /// Creates a new PeriodicReaderConfigurator
    pub fn new() -> Self {
        PeriodicReaderConfigurator {
            periodic_exporter_configurator: PeriodicExporterConfigurator::new(),
        }
    }

    /// Configures a periodic metrics reader based on the provided configuration
    pub fn configure(
        &self,
        metrics_configurator_manager: &MetricsConfiguratorManager,
        mut meter_provider_builder: opentelemetry_sdk::metrics::MeterProviderBuilder,
        config: &crate::model::metrics::reader::Periodic,
    ) -> Result<opentelemetry_sdk::metrics::MeterProviderBuilder, ConfiguratorError> {
        if let Some(exporter_config) = &config.exporter {
            meter_provider_builder = self.periodic_exporter_configurator.configure(
                metrics_configurator_manager,
                meter_provider_builder,
                exporter_config,
            )?;
        }

        Ok(meter_provider_builder)
    }
}

impl Default for PeriodicReaderConfigurator {
    fn default() -> Self {
        Self::new()
    }
}

/// Periodic exporter configurator
pub struct PeriodicExporterConfigurator {}

impl PeriodicExporterConfigurator {
    /// Creates a new PeriodicExporterConfigurator
    pub fn new() -> Self {
        PeriodicExporterConfigurator {}
    }

    /// Configures a periodic metrics exporter based on the provided configuration
    pub fn configure(
        &self,
        metrics_configurator_manager: &MetricsConfiguratorManager,
        mut meter_provider_builder: opentelemetry_sdk::metrics::MeterProviderBuilder,
        config: &crate::model::metrics::reader::PeriodicExporter,
    ) -> Result<opentelemetry_sdk::metrics::MeterProviderBuilder, ConfiguratorError> {
        if let Some(console_config) = &config.console {
            let configurator_option =
                metrics_configurator_manager.readers_periodic_exporter::<PeriodicExporterConsole>();
            if let Some(configurator) = configurator_option {
                meter_provider_builder =
                    configurator.configure(meter_provider_builder, console_config);
            } else {
                return Err(ConfiguratorError::NotRegisteredConfigurator("No configurator found for PeriodicExporterConsole. Make sure it is registered as configurator.".to_string()));
            }
        }

        if let Some(otlp_config) = &config.otlp {
            let configurator_option =
                metrics_configurator_manager.readers_periodic_exporter::<PeriodicExporterOtlp>();
            if let Some(configurator) = configurator_option {
                meter_provider_builder =
                    configurator.configure(meter_provider_builder, otlp_config);
            } else {
                return Err(ConfiguratorError::NotRegisteredConfigurator("No configurator found for PeriodicExporterOtlp. Make sure it is registered as configurator.".to_string()));
            }
        }

        Ok(meter_provider_builder)
    }
}

impl Default for PeriodicExporterConfigurator {
    fn default() -> Self {
        Self::new()
    }
}

// Pull reader configurator
pub struct PullReaderConfigurator {
    pull_exporter_configurator: PullExporterConfigurator,
}

impl PullReaderConfigurator {
    /// Creates a new PullReaderConfigurator
    pub fn new() -> Self {
        PullReaderConfigurator {
            pull_exporter_configurator: PullExporterConfigurator::new(),
        }
    }

    /// Configures a pull metrics reader based on the provided configuration
    pub fn configure(
        &self,
        metrics_configurator_manager: &MetricsConfiguratorManager,
        mut meter_provider_builder: opentelemetry_sdk::metrics::MeterProviderBuilder,
        config: &crate::model::metrics::reader::Pull,
    ) -> Result<opentelemetry_sdk::metrics::MeterProviderBuilder, ConfiguratorError> {
        if let Some(exporter_config) = &config.exporter {
            meter_provider_builder = self.pull_exporter_configurator.configure(
                metrics_configurator_manager,
                meter_provider_builder,
                exporter_config,
            )?;
        }
        Ok(meter_provider_builder)
    }
}

impl Default for PullReaderConfigurator {
    fn default() -> Self {
        Self::new()
    }
}

/// Pull exporter configurator
pub struct PullExporterConfigurator {}

impl PullExporterConfigurator {
    /// Creates a new PullExporterConfigurator
    pub fn new() -> Self {
        PullExporterConfigurator {}
    }

    pub fn configure(
        &self,
        _metrics_configurator_manager: &MetricsConfiguratorManager,
        meter_provider_builder: opentelemetry_sdk::metrics::MeterProviderBuilder,
        config: &crate::model::metrics::reader::PullExporter,
    ) -> Result<opentelemetry_sdk::metrics::MeterProviderBuilder, ConfiguratorError> {
        if let Some(_prometheus_config) = &config.prometheus {
            // Explicitly Prometheus exporter is not supported in this configurator.
            return Err(ConfiguratorError::UnsupportedExporter(
                "Prometheus exporter is not supported.".to_string(),
            ));
        }
        Ok(meter_provider_builder)
    }
}

impl Default for PullExporterConfigurator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {

    use opentelemetry_sdk::metrics::SdkMeterProvider;

    use crate::MetricsReaderPeriodicExporterConfigurator;

    use super::*;

    struct MockMetricsReadersPeriodicExporterConsoleConfigurator {}

    impl MockMetricsReadersPeriodicExporterConsoleConfigurator {
        fn new() -> Self {
            MockMetricsReadersPeriodicExporterConsoleConfigurator {}
        }

        fn register_into(manager: &mut crate::ConfiguratorManager) {
            manager
                .metrics_mut()
                .register_periodic_exporter_configurator::<PeriodicExporterConsole>(Box::new(
                    Self::new(),
                ));
        }
    }

    impl MetricsReaderPeriodicExporterConfigurator
        for MockMetricsReadersPeriodicExporterConsoleConfigurator
    {
        fn configure(
            &self,
            meter_provider_builder: opentelemetry_sdk::metrics::MeterProviderBuilder,
            _config: &dyn std::any::Any,
        ) -> opentelemetry_sdk::metrics::MeterProviderBuilder {
            // Mock implementation: just return the builder as is
            meter_provider_builder
        }

        fn as_any(&self) -> &dyn std::any::Any {
            todo!()
        }
    }

    #[test]
    fn test_reader_configurator_configure() {
        let configurator = ReaderConfigurator::new();
        let mut configurator_manager = crate::ConfiguratorManager::new();
        MockMetricsReadersPeriodicExporterConsoleConfigurator::register_into(
            &mut configurator_manager,
        );
        let meter_provider_builder = SdkMeterProvider::builder();

        let config = crate::model::metrics::reader::Reader::Periodic(
            crate::model::metrics::reader::Periodic {
                exporter: Some(crate::model::metrics::reader::PeriodicExporter {
                    console: Some(PeriodicExporterConsole { temporality: None }),
                    otlp: None,
                }),
            },
        );

        let metrics_configurator_manager = configurator_manager.metrics();

        _ = configurator
            .configure(
                metrics_configurator_manager,
                meter_provider_builder,
                &config,
            )
            .unwrap();
    }

    #[test]
    fn test_reader_configurator_configure_console_configurator_not_registered() {
        let configurator = ReaderConfigurator::new();
        let metrics_configurator_manager = MetricsConfiguratorManager::new();
        let meter_provider_builder = SdkMeterProvider::builder();

        let config = crate::model::metrics::reader::Reader::Periodic(
            crate::model::metrics::reader::Periodic {
                exporter: Some(crate::model::metrics::reader::PeriodicExporter {
                    console: Some(PeriodicExporterConsole { temporality: None }),
                    otlp: None,
                }),
            },
        );

        let result = configurator.configure(
            &metrics_configurator_manager,
            meter_provider_builder,
            &config,
        );
        if let Err(e) = result {
            assert!(e
                .to_string()
                .contains("No configurator found for PeriodicExporterConsole"));
        } else {
            panic!("Expected error due to missing configurator, but got Ok");
        }
    }

    #[test]
    fn test_reader_configurator_configure_otlp_configurator_not_registered() {
        let configurator = ReaderConfigurator::new();
        let metrics_configurator_manager = MetricsConfiguratorManager::new();
        let meter_provider_builder = SdkMeterProvider::builder();

        let config = crate::model::metrics::reader::Reader::Periodic(
            crate::model::metrics::reader::Periodic {
                exporter: Some(crate::model::metrics::reader::PeriodicExporter {
                    console: None,
                    otlp: Some(PeriodicExporterOtlp {
                        endpoint: None,
                        protocol: None,
                        temporality: None,
                    }),
                }),
            },
        );

        let result = configurator.configure(
            &metrics_configurator_manager,
            meter_provider_builder,
            &config,
        );
        if let Err(e) = result {
            assert!(e
                .to_string()
                .contains("No configurator found for PeriodicExporterOtlp"));
        } else {
            panic!("Expected error due to missing configurator, but got Ok");
        }
    }

    #[test]
    fn test_periodic_exporter_configurator_configure_unsupported_exporter() {
        let configurator = PullExporterConfigurator::new();
        let metrics_configurator_manager = MetricsConfiguratorManager::new();
        let meter_provider_builder = SdkMeterProvider::builder();
        let config = crate::model::metrics::reader::PullExporter {
            prometheus: Some(crate::model::metrics::reader::PullExporterPrometheus {
                host: "localhost".to_string(),
                port: 9090,
            }),
        };
        let result = configurator.configure(
            &metrics_configurator_manager,
            meter_provider_builder,
            &config,
        );
        if let Err(e) = result {
            assert!(e
                .to_string()
                .contains("Prometheus exporter is not supported."));
        } else {
            panic!("Expected error due to unsupported exporter, but got Ok");
        }
    }
}
