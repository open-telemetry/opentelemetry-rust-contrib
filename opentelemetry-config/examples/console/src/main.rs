//! # Example OpenTelemetry Config Console
//!
//! This example demonstrates how to configure OpenTelemetry Metrics
//! using the OpenTelemetry Config crate with a Mock Console Exporter.
//! It is helpful to implement and test custom exporters.

use opentelemetry_config::{
    configurators::TelemetryConfigurator, model::metrics::reader::PeriodicExporterConsole,
    ConfiguratorManager, MetricsReaderPeriodicExporterConfigurator,
};
use opentelemetry_sdk::metrics::Temporality;
use opentelemetry_sdk::{
    error::OTelSdkResult,
    metrics::{data::ResourceMetrics, exporter::PushMetricExporter, MeterProviderBuilder},
};
use std::time::Duration;

use std::env;

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    if args.len() == 1 || (args.len() > 1 && args[1] == "--help") {
        println!("Usage: cargo run -- --file ../metrics_console.yaml");
        println!("This example demonstrates how to configure OpenTelemetry Metrics using the OpenTelemetry Config crate with a Console Exporter.");
        return Ok(());
    }
    if args.len() < 3 || args[1] != "--file" {
        println!("Error: Configuration file path not provided.");
        println!("Usage: cargo run -- --file ../metrics_console.yaml");
        return Ok(());
    }
    let config_file = &args[2];

    // Setup configurator manager with console exporter configurator
    let configurator = Box::new(MockPeriodicExporterConfigurator::new());
    let mut configurator_manager = ConfiguratorManager::new();

    // Register the console exporter configurator for the specific exporter type.
    configurator_manager
        .metrics_mut()
        .register_periodic_exporter_configurator::<PeriodicExporterConsole>(configurator);

    let telemetry_configurator = TelemetryConfigurator::new();
    let providers = telemetry_configurator
        .configure_from_yaml_file(&configurator_manager, config_file)
        .unwrap();
    assert!(providers.meter_provider().is_some());
    println!("Metrics configured with Console Exporter successfully.");

    println!(
        "Meter provider configured: {}",
        providers.meter_provider().is_some()
    );
    println!(
        "Logs provider configured: {}",
        providers.logs_provider().is_some()
    );
    println!(
        "Traces provider configured: {}",
        providers.traces_provider().is_some()
    );

    println!("Shutting down telemetry providers...");
    providers.shutdown()?;
    Ok(())
}

pub struct MockPeriodicExporterConfigurator {}

impl MockPeriodicExporterConfigurator {
    fn new() -> Self {
        Self {}
    }
}

pub struct MockConsoleExporter {
    temporality: Temporality,
}

impl MockConsoleExporter {
    fn new() -> Self {
        let temporality = Temporality::Delta;
        Self { temporality }
    }

    pub fn set_temporality(&mut self, temporality: Temporality) {
        self.temporality = temporality;
    }
}

impl PushMetricExporter for MockConsoleExporter {
    async fn export(&self, metrics: &ResourceMetrics) -> OTelSdkResult {
        println!(
            "MockConsoleExporter exporting metrics {:?} with temporality: {:?}",
            metrics, self.temporality
        );
        Ok(())
    }

    /// Flushes any metric data held by an exporter.
    fn force_flush(&self) -> OTelSdkResult {
        println!("MockConsoleExporter force flushing metrics.");
        Ok(())
    }

    /// Releases any held computational resources.
    ///
    /// After Shutdown is called, calls to Export will perform no operation and
    /// instead will return an error indicating the shutdown state.
    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        println!(
            "MockConsoleExporter shutting down with timeout: {:?}",
            timeout
        );
        Ok(())
    }

    /// Shutdown with the default timeout of 5 seconds.
    fn shutdown(&self) -> OTelSdkResult {
        self.shutdown_with_timeout(Duration::from_secs(5))
    }

    /// Access the [Temporality] of the MetricExporter.
    fn temporality(&self) -> Temporality {
        self.temporality
    }
}

impl MetricsReaderPeriodicExporterConfigurator for MockPeriodicExporterConfigurator {
    fn configure(
        &self,
        mut meter_provider_builder: MeterProviderBuilder,
        config: &dyn std::any::Any,
    ) -> MeterProviderBuilder {
        let mut exporter = MockConsoleExporter::new();

        let config = config
            .downcast_ref::<PeriodicExporterConsole>()
            .expect("Invalid config type. Expected PeriodicExporterConsole.");

        if let Some(temporality) = &config.temporality {
            match temporality {
                opentelemetry_config::model::metrics::reader::Temporality::Delta => {
                    exporter.set_temporality(Temporality::Delta);
                }
                opentelemetry_config::model::metrics::reader::Temporality::Cumulative => {
                    exporter.set_temporality(Temporality::Cumulative);
                }
            }
        }

        meter_provider_builder = meter_provider_builder.with_periodic_exporter(exporter);
        meter_provider_builder
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
