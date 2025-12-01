//! # Example OpenTelemetry Config Custom exporter
//!
//! This example demonstrates how to configure OpenTelemetry Metrics
//! using the OpenTelemetry Config crate with a Mock Custom Exporter.
//! It is helpful to implement and test custom exporters.

use opentelemetry_config::{
    providers::TelemetryProviders, ConfigurationError, ConfigurationProviderRegistry,
};

use opentelemetry_sdk::{
    error::OTelSdkResult,
    metrics::{
        data::ResourceMetrics, exporter::PushMetricExporter, MeterProviderBuilder, PeriodicReader,
    },
};
use std::time::Duration;
use std::{env, fs};

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    if args.len() == 1 || (args.len() > 1 && args[1] == "--help") {
        println!("Usage: cargo run -- --file metrics_custom.yaml");
        println!("This example demonstrates how to configure OpenTelemetry Metrics using the OpenTelemetry Config crate with a custom Exporter.");
        return Ok(());
    }
    if args.len() < 3 || args[1] != "--file" {
        println!("Error: Configuration file path not provided.");
        println!("Usage: cargo run -- --file metrics_custom.yaml");
        return Ok(());
    }

    let config_file = &args[2];
    let config_yaml = match fs::read_to_string(config_file) {
        Ok(content) => content,
        Err(e) => {
            println!(
                "Error: Unable to read the configuration file at path: {}: {}",
                config_file, e
            );
            return Ok(());
        }
    };

    // Setup configuration registry with custom exporter provider.
    let mut registry = ConfigurationProviderRegistry::default();

    // Register the custom exporter provider.
    MockPeriodicExporterProvider::register_into(&mut registry);

    // Configure telemetry from the provided YAML file.
    let providers = match TelemetryProviders::configure_from_yaml_str(&registry, &config_yaml) {
        Ok(providers) => providers,
        Err(e) => {
            println!(
                "Error: Failed to configure telemetry from the provided YAML file: {}",
                e
            );
            return Ok(());
        }
    };

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

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PeriodicConfig {
    pub interval: Option<u64>,
    pub exporter: MockCustomConfig,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MockCustomConfig {
    pub custom: CustomData,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomData {
    pub custom_string_field: String,
    pub custom_int_field: i32,
}

struct MockPeriodicExporterProvider {}

impl MockPeriodicExporterProvider {
    pub fn register_into(
        registry: &mut opentelemetry_config::ConfigurationProviderRegistry,
    ) {
        registry.register_metric_exporter_factory(
            "custom",
            MockPeriodicExporterProvider::register_mock_exporter_factory,
        );
    }

    pub fn register_mock_exporter_factory(
        mut meter_provider_builder: MeterProviderBuilder,
        periodic_config_str: &str,
    ) -> Result<MeterProviderBuilder, ConfigurationError> {
        let periodic_config: PeriodicConfig =
            serde_yaml::from_str(periodic_config_str).map_err(|e| {
                ConfigurationError::InvalidConfiguration(format!(
                    "Failed to parse PeriodicConfig: {}",
                    e
                ))
            })?;

        let exporter_config = periodic_config.exporter;

        println!(
            "Configuring MockCustomExporter with string field: {} and int field: {}",
            exporter_config.custom.custom_string_field, exporter_config.custom.custom_int_field
        );

        let exporter = MockCustomExporter {
            custom_config: exporter_config,
        };

        let interval_millis = periodic_config.interval.unwrap_or(60000);

        // TODO: Add timeout from config
        let reader = PeriodicReader::builder(exporter)
            .with_interval(std::time::Duration::from_millis(interval_millis))
            .build();

        meter_provider_builder = meter_provider_builder.with_reader(reader);
        Ok(meter_provider_builder)
    }
}

struct MockCustomExporter {
    custom_config: MockCustomConfig,
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
