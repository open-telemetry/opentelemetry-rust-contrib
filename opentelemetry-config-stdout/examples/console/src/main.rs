//! # Example OpenTelemetry Config Console
//!
//! This example demonstrates how to configure OpenTelemetry Metrics
//! using the OpenTelemetry Config crate with a Console Exporter.

use opentelemetry_config::{providers::TelemetryProviders, ConfigurationProviderRegistry};

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

    // Setup configuration registry with console exporter provider.
    let mut configuration_providers_registry = ConfigurationProviderRegistry::default();
    let metrics_registry = configuration_providers_registry.metrics();
    metrics_registry.register_periodic_reader_factory(
        "console",
        opentelemetry_config_stdout::register_console_meter_reader_factory,
    );

    let providers = TelemetryProviders::configure_from_yaml_file(
        &configuration_providers_registry,
        config_file,
    )?;

    if let Some(meter_provider) = providers.meter_provider() {
        println!("Meter provider is configured. Shutting it down...");
        meter_provider.shutdown()?;
    } else {
        println!("No Meter Provider configured.");
    }

    if let Some(logs_provider) = providers.logs_provider() {
        println!("Logs provider is configured. Shutting it down...");
        logs_provider.shutdown()?;
    } else {
        println!("No Logs Provider configured.");
    }

    if let Some(traces_provider) = providers.traces_provider() {
        println!("Traces provider is configured. Shutting it down...");
        traces_provider.shutdown()?;
    } else {
        println!("No Traces Provider configured.");
    }

    Ok(())
}
