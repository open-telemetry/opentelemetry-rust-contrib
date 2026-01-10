# OpenTelemetry Declarative Configuration

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

| Status        |           |
| ------------- |-----------|
| Stability     | alpha     |
| Owners        | [Andres Borja](https://github.com/andborja) |

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
        interval: 60000  # milliseconds
        timeout: 30000   # milliseconds
        exporter:
          custom:
            custom_string_field: "my-custom-value"
            custom_int_field: 42

resource:
  service.name: "my-service"
  service.version: "1.0.0"
```

### 2. Implement a Periodic Reader Factory

See the complete implementation example in [examples/custom/src/main.rs](examples/custom/src/main.rs).

### 3. Register and Load Configuration

```rust
use opentelemetry_config::{
    ConfigurationProviderRegistry,
    providers::TelemetryProviders,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a configuration registry
    let mut registry = ConfigurationProviderRegistry::default();
    
    // Register the metric exporter factory function
    registry.register_metric_exporter_factory(
        "custom",
        create_custom_reader,
    );

    let yaml = "<yaml configuration>";

    // Load configuration from YAML string
    let providers = TelemetryProviders::configure_from_yaml(
        &registry,
        yaml
    )?;

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

- **`ConfigurationProviderRegistry`**: Central registry for configuration providers across all telemetry signals
- **`TelemetryProviders`**: Holds configured meter, tracer, and logger providers with static `configure_from_yaml` method
- **`MeterProviderFactory`**: Type alias for factory functions: `Fn(MeterProviderBuilder, &str) -> Result<MeterProviderBuilder, ConfigurationError>`
- **`ConfigurationError`**: Error type for configuration and registration failures
- **`ProviderError`**: Error type for provider-related failures

### Design Pattern

This crate follows a **factory-based decoupled implementation pattern**:

- **Centralized Configuration Model**: The configuration schema (YAML structure and data models) is defined and maintained centrally in this crate, ensuring alignment with the OpenTelemetry Configuration Standard. The general structure (metrics, traces, logs, resource) is enforced to maintain compatibility.
- **Extensible Configuration**: While the top-level structure is controlled, exporter-specific configurations are fully extensible. Factory functions can define their own configuration schemas that are deserialized from the YAML at runtime, enabling custom properties without modifying the core model.
- **Decoupled Implementations**: Actual exporter implementations live in external crates or user code, allowing the community to contribute custom exporters without modifying the core configuration model. Each factory function handles its own configuration deserialization and exporter instantiation.
- **Factory Function Pattern**: Periodic readers are registered via factory functions (`Fn(MeterProviderBuilder, &str) -> Result<MeterProviderBuilder, ConfigurationError>`) that receive the meter provider builder and YAML configuration string, allowing them to deserialize custom config structures and create readers with custom exporters.
- **Registry-Based Discovery**: A central registry maps exporter names (strings) to their factory functions, enabling dynamic configuration. Exporter names from the YAML configuration (e.g., `console`, `otlp`, `custom`) are used to look up the appropriate factory.
- **Community Control**: By keeping the top-level configuration model centralized and standardized, the community maintains consistency across all implementations while enabling complete flexibility for exporter-specific configurations.

This design enables:
- **Standard Compliance**: All configurations follow the official OpenTelemetry schema at the top level
- **Easy Extension**: Contributors can add new exporters with custom configurations by implementing factory functions in their own crates
- **Configuration Flexibility**: Each exporter can define its own configuration structure without requiring changes to the core crate
- **Version Independence**: Exporter implementations and their configurations can evolve independently from the core configuration schema
- **Mixed Exporters**: Users can combine official and custom exporters using the same configuration format
- **Type Safety**: Strong typing throughout the configuration pipeline with runtime validation and deserialization errors

### Configuration Model

The configuration is structured around the `Telemetry` model which includes:

- **`metrics`**: Metrics configuration including readers and exporters
- **`traces`**: (Coming soon) Trace configuration
- **`logs`**: (Coming soon) Log configuration
- **`resource`**: Resource attributes (service name, version, etc.)

## Examples

### Custom Exporter Example

See the [examples/custom](examples/custom) directory for a complete working example that demonstrates:

- Implementing a custom exporter with `PushMetricExporter` trait
- Defining a custom configuration structure
- Creating a factory function that deserializes config and creates the exporter
- Registering the factory with the configuration registry
- Loading configuration from a YAML file
- Proper shutdown handling

To run the example:

```bash
cd examples/custom
cargo run -- --file ../metrics_custom.yaml
```

## Configuration Schema

### Metrics Configuration

```yaml
metrics:
  readers:
    - periodic:
        interval: 60000  # Export interval in milliseconds (default: 60000)
        timeout: 30000   # Export timeout in milliseconds (default: 30000)
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

## Current Limitations

- Only metrics configuration is currently implemented
- Traces and logs configuration are planned for future releases

## Contributing

Contributions are welcome! Please feel free to submit issues or pull requests.

## License

This project is licensed under the Apache-2.0 license.

## Release Notes

You can find the release notes (changelog) [here](CHANGELOG.md).
