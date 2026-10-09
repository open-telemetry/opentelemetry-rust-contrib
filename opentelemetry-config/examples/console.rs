//! # Example: OpenTelemetry Declarative Configuration with Console Exporter
//!
//! This example demonstrates loading an OpenTelemetry declarative configuration
//! file adhering to the v1.2.0 schema, initializing an `SdkMeterProvider` with a
//! built-in standard periodic `console` exporter, recording metrics, flushing
//! them to stdout, and shutting down cleanly.

use opentelemetry::{metrics::MeterProvider as _, KeyValue};
use opentelemetry_config::{providers::TelemetryProviders, ConfigurationProviderRegistry};
use std::error::Error;

const YAML_CONFIG: &str = r#"
file_format: "1.2"
resource:
  attributes:
    - name: service.name
      value: "console-metric-example"
    - name: service.version
      value: "1.0.0"
    - name: deployment.environment
      value: "demonstration"
meter_provider:
  readers:
    - periodic:
        interval: 5000
        exporter:
          console:
            temporality_preference: cumulative
"#;

fn main() -> Result<(), Box<dyn Error>> {
    println!("=== Loading declarative OpenTelemetry YAML configuration ===");

    // 1. Create a configuration provider registry
    let registry = ConfigurationProviderRegistry::default();

    // 2. Configure telemetry providers from declarative YAML string
    let providers = TelemetryProviders::configure_from_yaml_str(&registry, YAML_CONFIG)?;

    // 3. Obtain the initialized MeterProvider
    let meter_provider = providers
        .meter_provider()
        .ok_or("Expected MeterProvider to be configured")?;

    println!("=== Initialized SdkMeterProvider successfully ===");

    // 4. Create a meter and a metric instrument
    let meter = meter_provider.meter("example_meter");
    let work_items_counter = meter
        .u64_counter("work_items_total")
        .with_description("Total number of processed work items")
        .with_unit("items")
        .build();

    println!("=== Recording metrics ===");

    // 5. Record measurements
    work_items_counter.add(
        42,
        &[
            KeyValue::new("worker.id", "worker-42"),
            KeyValue::new("status", "success"),
        ],
    );

    println!("=== Flushing MeterProvider (exporting to console) ===");

    // 6. Flush the provider so the periodic reader immediately exports recorded metrics to console
    meter_provider.force_flush()?;

    println!("=== Shutting down MeterProvider ===");

    // 7. Clean shutdown
    meter_provider.shutdown()?;

    println!("=== Completed successfully ===");
    Ok(())
}
