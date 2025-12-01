//! # Example of an application using declarative configuration as part of their own configuration.
//!
//! This example demonstrates how to configure OpenTelemetry Metrics using the OpenTelemetry Config crate
//! with a custom Exporter within an application.

pub mod mock_provider;
pub mod model;
use std::env;

use crate::mock_provider::MockPeriodicExporterProvider;
use opentelemetry_config::{providers::TelemetryProviders, ConfigurationProviderRegistry};

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    if args.len() == 1 || (args.len() > 1 && args[1] == "--help") {
        println!("Usage: cargo run -- --file application.yaml");
        println!("This example demonstrates how to configure OpenTelemetry Metrics using the OpenTelemetry Config crate with a custom Exporter within an application.");
        return Ok(());
    }
    if args.len() < 3 || args[1] != "--file" {
        println!("Error: Configuration file path not provided.");
        println!("Usage: cargo run -- --file application.yaml");
        return Ok(());
    }
    let config_file = &args[2];

    // Load application specific configuration
    let file = match std::fs::File::open(config_file) {
        Ok(f) => f,
        Err(e) => {
            eprintln!(
                "Error: Unable to open configuration file '{}': {}",
                config_file, e
            );
            return Ok(());
        }
    };
    let app_config: model::Application = match serde_yaml::from_reader(file) {
        Ok(config) => config,
        Err(e) => {
            eprintln!(
                "Error: Failed to load configuration from '{}': {}",
                config_file, e
            );
            return Ok(());
        }
    };

    // Configure the application itself
    println!(
        "Loaded application configuration version {}",
        app_config.version
    );

    // Setup configuration registry with custom providers.
    let registry = initialize_telemetry_registry();

    // Configure telemetry from the application configuration.
    let telemetry_config_yaml = serde_yaml::to_string(&app_config.service.telemetry)?;
    let telemetry_providers = match configure_telemetry(&registry, &telemetry_config_yaml) {
        Ok(providers) => providers,
        Err(e) => {
            println!(
                "Error: Failed to configure telemetry from the application configuration: {}",
                e
            );
            return Ok(());
        }
    };

    // Use and verify the telemetry configuration
    if let Some(meter_provider) = telemetry_providers.meter_provider() {
        println!("Meter provider configured successfully. Shutting it down...");
        meter_provider.shutdown()?;
    } else {
        println!("No Meter provider configured.");
    }

    if let Some(logs_provider) = telemetry_providers.logs_provider() {
        println!("Logs provider configured successfully. Shutting it down...");
        logs_provider.shutdown()?;
    } else {
        println!("No Logs provider configured.");
    }

    if let Some(traces_provider) = telemetry_providers.traces_provider() {
        println!("Traces provider configured successfully. Shutting it down...");
        traces_provider.shutdown()?;
    } else {
        println!("No Traces provider configured.");
    }
    Ok(())
}

/// Initializes the telemetry configuration registry with custom providers.
fn initialize_telemetry_registry() -> ConfigurationProviderRegistry {
    let mut registry = ConfigurationProviderRegistry::default();

    // Register the custom exporter factory.
    registry.register_metric_exporter_factory(
        "custom",
        MockPeriodicExporterProvider::register_mock_exporter_factory,
    );

    registry
}

/// Configures telemetry providers from the given telemetry configuration.
fn configure_telemetry(
    registry: &ConfigurationProviderRegistry,
    telemetry_config: &str,
) -> Result<TelemetryProviders, Box<dyn std::error::Error>> {
    let providers = TelemetryProviders::configure_from_yaml_str(&registry, telemetry_config)?;
    Ok(providers)
}
