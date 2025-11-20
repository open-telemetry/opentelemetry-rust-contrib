//! # Example of an application using declarative configuration as part of their own configuration.
//!
//! This example demonstrates how to configure OpenTelemetry Metrics using the OpenTelemetry Config crate
//! with a custom Exporter within an application.

pub mod model;
pub mod mock_provider;

use opentelemetry_config::{
    RegistryKey,
    model::Telemetry,
    providers::TelemetryProviders,
    ConfigurationProviderRegistry,
};

use std::env;

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
    let app_config: model::Application = serde_yaml::from_reader(std::fs::File::open(config_file)?)?;

    // Configure the application itself
    println!(
        "Loaded application configuration version {}",
        app_config.version
    );

    // Setup configuration registry with custom providers.
    let registry = initialize_telemetry_registry();

    // Configure telemetry from the application configuration.
    let telemetry_providers = configure_telemetry(&registry, &app_config.service.telemetry)?;

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

    let key = crate::RegistryKey::ReadersPeriodicExporter("custom".to_string());
    // Register the custom exporter provider.
    registry.register_meter_provider_factory(
        key,
        mock_provider::MockPeriodicReaderProvider::register_mock_reader_factory,
    );

    registry
}

/// Configures telemetry providers from the given telemetry configuration.
fn configure_telemetry(registry: &ConfigurationProviderRegistry, telemetry_config: &Telemetry) -> Result<TelemetryProviders, Box<dyn std::error::Error>> {
    let providers = TelemetryProviders::configure(&registry, telemetry_config)?;
    Ok(providers)
}
