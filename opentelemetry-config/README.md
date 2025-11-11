# OpenTelemetry Declarative Configuration

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

Declarative configuration for applications instrumented with [`OpenTelemetry`]. 

[`OpenTelemetry`]: https://crates.io/crates/opentelemetry

## Overview

This crate provides a declarative, YAML-based configuration approach for the OpenTelemetry Rust SDK. Instead of programmatically building telemetry providers with code, you can define your OpenTelemetry configuration in YAML files and load them at runtime.

The configuration model is aligned with the [OpenTelemetry Configuration Schema](https://github.com/open-telemetry/opentelemetry-configuration), following the standard defined in the [kitchen-sink.yaml](https://github.com/open-telemetry/opentelemetry-configuration/blob/main/examples/kitchen-sink.yaml) example. This ensures compatibility and consistency with OpenTelemetry implementations across different languages and platforms.

### Features

- **Declarative Configuration**: Define metrics, traces, and logs configuration in YAML
- **Extensible Architecture**: Register custom providers for different exporters
- **Type-Safe**: Strongly typed configuration models with serde deserialization
- **Multiple Exporters**: Support for Console, OTLP, and custom exporters
- **Resource Attributes**: Configure resource attributes for all telemetry signals

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
opentelemetry-config = "0.1.0"
```

## Quick Start

### 1. Create a YAML Configuration File

Create a file named `otel-config.yaml`:

```yaml
metrics:
  readers:
    - periodic:
        exporter:
          console:
            temporality: delta

resource:
  service.name: "my-service"
  service.version: "1.0.0"
```

### 2. Load and Apply Configuration

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a configuration registry and register exporters
    let mut registry = ConfigurationProvidersRegistry::new();
    registry
        .metrics_mut()
        .register_periodic_exporter_provider::<PeriodicExporterConsole>(
            Box::new(ConsoleExporterProvider)
        );

    // Load configuration from YAML file
    let telemetry_provider = TelemetryProvider::new();
    let providers = telemetry_provider
        .provide_from_yaml_file(&registry, "otel-config.yaml")?;

    // Use the configured providers
    if let Some(meter_provider) = providers.meter_provider() {
        // Your application code here

        // Shutdown the meter provider
        meter_provider.shutdown()?;
    }

    Ok(())
}
```

## Architecture

### Core Components

- **`ConfigurationProvidersRegistry`**: Central registry for configuration providers
- **`TelemetryProvider`**: Orchestrates the configuration process from YAML to providers
- **`TelemetryProviders`**: Holds configured meter, tracer, and logger providers
- **`MetricsReaderPeriodicExporterProvider`**: Trait for implementing custom metric exporters

### Design Pattern

This crate follows a **decoupled implementation pattern**:

- **Centralized Configuration Model**: The configuration schema (YAML structure and data models) is defined and maintained centrally in this crate, ensuring alignment with the OpenTelemetry Configuration Standard
- **Decoupled Implementations**: Actual exporter implementations live in external crates, allowing the community to contribute custom exporters without modifying the core configuration model
- **Community Control**: By keeping the configuration model centralized and standardized, the community maintains consistency across all implementations while enabling extensibility

This design enables:
- **Standard Compliance**: All configurations follow the official OpenTelemetry schema
- **Easy Extension**: Contributors can add new exporters by implementing traits in their own crates
- **Version Independence**: Exporter implementations can evolve independently from the configuration schema
- **Flexibility**: Users can mix official and custom exporters using the same configuration format

### Configuration Model

The configuration is structured around the `Telemetry` model which includes:

- **`metrics`**: Metrics configuration including readers and exporters
- **`traces`**: (Coming soon) Trace configuration
- **`logs`**: (Coming soon) Log configuration
- **`resource`**: Resource attributes (service name, version, etc.)

## Examples

### Console Exporter Example

See the [examples/console](examples/console) directory for a complete working example that demonstrates:

- Setting up a console exporter provider
- Loading configuration from a YAML file
- Configuring a meter provider
- Proper shutdown handling

To run the example:

```bash
cd examples/console
cargo run -- --file ../metrics_console.yaml
```

## Configuration Schema

### Metrics Configuration

```yaml
metrics:
  readers:
    - periodic:
        exporter:
          console:
            temporality: delta  # or cumulative
          # or
          otlp:
            endpoint: "http://localhost:4317"
            protocol: grpc
```

### Resource Attributes

```yaml
resource:
  service.name: "my-service"
  service.version: "1.0.0"
  deployment.environment: "production"
  # Add any custom attributes
```

## Extending with Custom Exporters

To add support for a custom exporter:

1. Define your exporter configuration model:

```rust
#[derive(Debug, Deserialize)]
pub struct MyCustomExporter {
    pub endpoint: String,
    pub timeout: Option<u64>,
}
```

2. Implement the provider trait:

```rust
impl MetricsReaderPeriodicExporterProvider for MyCustomProvider {
    fn provide(
        &self,
        meter_provider_builder: MeterProviderBuilder,
        config: &dyn std::any::Any,
    ) -> MeterProviderBuilder {
        let config = config.downcast_ref::<PeriodicExporterConsole>()
            .expect("Invalid config type");
        
        // Build your exporter with the config
        let exporter = MyExporter::new(config);
        meter_provider_builder.with_periodic_exporter(exporter)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
```

3. Register it with the ConfigurationProvidersRegistry:

```rust
registry
    .metrics_mut()
    .register_periodic_exporter_provider::<PeriodicExporterConsole>(
        Box::new(MyCustomProvider)
    );
```

## Current Limitations

- Only metrics configuration is currently implemented
- Traces and logs configuration are planned for future releases

## Contributing

Contributions are welcome! Please feel free to submit issues or pull requests.

## License

This project is licensed under the Apache-2.0 license.

## Release Notes

You can find the release notes (changelog) [here](https://github.com/open-telemetry/opentelemetry-rust-contrib/tree/main/opentelemetry-config/CHANGELOG.md).
