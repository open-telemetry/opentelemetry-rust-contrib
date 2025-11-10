//! # Example OpenTelemetry Config Console
//!
//! This example demonstrates how to configure OpenTelemetry Metrics
//! using the OpenTelemetry Config crate with a Console Exporter.

use opentelemetry_config::{configurators::TelemetryConfigurator, ConfiguratorManager};
use opentelemetry_config_stdout::ConsolePeriodicExporterConfigurator;

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
    let mut configurator_manager = ConfiguratorManager::new();
    ConsolePeriodicExporterConfigurator::register_into(&mut configurator_manager);

    let telemetry_configurator = TelemetryConfigurator::new();
    let providers = telemetry_configurator
        .configure_from_yaml_file(&configurator_manager, config_file)?;

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
