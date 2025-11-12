//! # Example OpenTelemetry Config Custom exporter
//!
//! This example demonstrates how to configure OpenTelemetry Metrics
//! using the OpenTelemetry Config crate with a Mock Custom Exporter.
//! It is helpful to implement and test custom exporters.

use opentelemetry_config::{
    providers::TelemetryProvider, ConfigurationProvidersRegistry, MetricsExporterId,
    MetricsReaderPeriodicExporterProvider,
};
use opentelemetry_sdk::{
    error::OTelSdkResult,
    metrics::{data::ResourceMetrics, exporter::PushMetricExporter, MeterProviderBuilder},
};
use serde_yaml::Value;
use std::env;
use std::time::Duration;

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    if args.len() == 1 || (args.len() > 1 && args[1] == "--help") {
        println!("Usage: cargo run -- --file ../metrics_custom.yaml");
        println!("This example demonstrates how to configure OpenTelemetry Metrics using the OpenTelemetry Config crate with a custom Exporter.");
        return Ok(());
    }
    if args.len() < 3 || args[1] != "--file" {
        println!("Error: Configuration file path not provided.");
        println!("Usage: cargo run -- --file ../metrics_custom.yaml");
        return Ok(());
    }
    let config_file = &args[2];

    // Setup configuration registry with custom exporter provider.
    let mut registry = ConfigurationProvidersRegistry::new();

    // Register the custom exporter provider.
    MockPeriodicExporterProvider::register_into(&mut registry);

    let telemetry_provider = TelemetryProvider::new();
    let providers = telemetry_provider
        .provide_from_yaml_file(&registry, config_file)
        .unwrap();

    if let Some(meter_provider) = providers.meter_provider() {
        println!("Meter provider configured successfully. Shutting it down...");
        meter_provider.shutdown()?;
    } else {
        println!("No Meter provider configured.");
    }

    if let Some(logs_provider) = providers.logs_provider() {
        println!("Logs provider configured successfully. Shutting it down...");
        logs_provider.shutdown()?;
    } else {
        println!("No Logs provider configured.");
    }

    if let Some(traces_provider) = providers.traces_provider() {
        println!("Traces provider configured successfully. Shutting it down...");
        traces_provider.shutdown()?;
    } else {
        println!("No Traces provider configured.");
    }

    Ok(())
}

pub struct MockPeriodicExporterProvider {}

impl MockPeriodicExporterProvider {
    fn new() -> Self {
        Self {}
    }

    pub fn register_into(registry: &mut ConfigurationProvidersRegistry) {
        let key = MetricsExporterId::PeriodicExporter.qualified_name("custom");
        registry
            .metrics_mut()
            .register_periodic_exporter_provider(key, Box::new(Self::new()));
    }
}

pub struct MockCustomExporter {
    custom_config: Option<MockCustomConfig>,
}

impl MockCustomExporter {
    fn new() -> Self {
        Self {
            custom_config: None,
        }
    }

    pub fn set_custom_config(&mut self, custom_config: MockCustomConfig) {
        self.custom_config = Some(custom_config);
    }
}

impl PushMetricExporter for MockCustomExporter {
    async fn export(&self, metrics: &ResourceMetrics) -> OTelSdkResult {
        println!(
            "MockCustomExporter exporting metrics {:?} with custom config: {:?}",
            metrics, self.custom_config
        );
        Ok(())
    }

    fn force_flush(&self) -> OTelSdkResult {
        println!("MockCustomExporter force flushing metrics.");
        Ok(())
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        println!(
            "MockCustomExporter shutting down with timeout: {:?}",
            timeout
        );
        Ok(())
    }

    fn shutdown(&self) -> OTelSdkResult {
        self.shutdown_with_timeout(Duration::from_secs(5))
    }

    fn temporality(&self) -> opentelemetry_sdk::metrics::Temporality {
        opentelemetry_sdk::metrics::Temporality::Cumulative
    }
}

#[derive(serde::Deserialize, Debug)]
pub struct MockCustomConfig {
    pub custom_string_field: String,
    pub custom_int_field: i32,
}

impl MetricsReaderPeriodicExporterProvider for MockPeriodicExporterProvider {
    fn provide(
        &self,
        mut meter_provider_builder: MeterProviderBuilder,
        config: &Value,
    ) -> MeterProviderBuilder {
        let mut exporter = MockCustomExporter::new();

        let config = serde_yaml::from_value::<MockCustomConfig>(config.clone())
            .expect("Failed to deserialize MockCustomConfig");
        println!(
            "Configuring MockCustomExporter with string field: {} and int field: {}",
            config.custom_string_field, config.custom_int_field
        );

        exporter.set_custom_config(config);

        meter_provider_builder = meter_provider_builder.with_periodic_exporter(exporter);
        meter_provider_builder
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
