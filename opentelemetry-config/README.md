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
          custom:
            temporality: delta

resource:
  service.name: "my-service"
  service.version: "1.0.0"
```

### 2. Implement an Exporter Factory

```rust
use opentelemetry_config::{ConfigurationProvidersRegistry, ConfigurationError};
use opentelemetry_sdk::{
    error::OTelSdkResult,
    metrics::{MeterProviderBuilder, data::ResourceMetrics, exporter::PushMetricExporter},
};
use serde_yaml::Value;
use std::time::Duration;

// Define your custom configuration model
#[derive(Debug, serde::Deserialize)]
pub struct CustomConfig {
    pub custom_string_field: String,
    pub custom_int_field: i32,
}

// Implement your custom exporter
pub struct CustomExporter {
    config: Option<CustomConfig>,
}

impl CustomExporter {
    fn new() -> Self {
        Self { config: None }
    }

    pub fn set_config(&mut self, config: CustomConfig) {
        self.config = Some(config);
    }
}

impl PushMetricExporter for CustomExporter {
  // PushMetricExporter methods...
}

// Factory function that creates and configures the exporter
fn register_custom_exporter(
    mut builder: MeterProviderBuilder,
    config: &Value,
) -> Result<MeterProviderBuilder, ConfigurationError> {
    let mut exporter = CustomExporter::new();

    // Deserialize your custom config from YAML
    let custom_config = serde_yaml::from_value::<CustomConfig>(config.clone())
        .map_err(|e| ConfigurationError::InvalidConfiguration(e.to_string()))?;

    exporter.set_config(custom_config);
    builder = builder.with_periodic_exporter(exporter);

    Ok(builder)
}
```

### 3. Register and Use Configuration

```rust
use opentelemetry_config::providers::TelemetryProvider;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a configuration registry
    let mut registry = ConfigurationProvidersRegistry::new();
    
    // Register the custom exporter factory function
    registry
        .metrics_mut()
        .register_periodic_exporter_factory(
            "custom".to_string(),
            register_custom_exporter,
        );

    // Load configuration from YAML file
    let telemetry_provider = TelemetryProvider::new();
    let providers = telemetry_provider
        .configure_from_yaml_file(&registry, "otel-config.yaml")?;

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

- **`ConfigurationProvidersRegistry`**: Central registry for configuration providers across all telemetry signals
- **`MetricsProvidersRegistry`**: Registry specifically for metrics exporter factory functions
- **`TelemetryProvider`**: Orchestrates the configuration process from YAML to SDK providers
- **`TelemetryProviders`**: Holds configured meter, tracer, and logger providers
- **`MetricConfigFactory`**: Type alias for factory functions that create and configure metric exporters
- **`ConfigurationError`**: Error type for configuration and registration failures

### Design Pattern

This crate follows a **factory-based decoupled implementation pattern**:

- **Centralized Configuration Model**: The configuration schema (YAML structure and data models) is defined and maintained centrally in this crate, ensuring alignment with the OpenTelemetry Configuration Standard. The general structure (metrics, traces, logs, resource) is enforced to maintain compatibility.
- **Extensible Configuration**: While the top-level structure is controlled, exporter-specific configurations are fully extensible. Factory functions can define their own configuration schemas that are deserialized from the YAML at runtime, enabling custom properties without modifying the core model.
- **Decoupled Implementations**: Actual exporter implementations live in external crates or user code, allowing the community to contribute custom exporters without modifying the core configuration model. Each factory function handles its own configuration deserialization and exporter instantiation.
- **Factory Function Pattern**: Exporters are registered via factory functions (`Fn(MeterProviderBuilder, &Value) -> Result<MeterProviderBuilder, ConfigurationError>`) that receive the meter provider builder and YAML configuration, allowing them to deserialize into any custom configuration structure they need.
- **Registry-Based Discovery**: A central registry maps exporter names to their factory functions, enabling dynamic configuration. Exporter names from the YAML are used as registry keys to look up the appropriate factory.
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

### 1. Define your exporter configuration model (optional):

```rust
#[derive(Debug, serde::Deserialize)]
pub struct MyCustomConfig {
    pub endpoint: String,
    pub timeout: Option<u64>,
}
```

### 2. Implement your exporter with `PushMetricExporter`:

```rust
use opentelemetry_sdk::{
    error::OTelSdkResult,
    metrics::{data::ResourceMetrics, exporter::PushMetricExporter},
};

pub struct MyCustomExporter {
    config: MyCustomConfig,
}

impl MyCustomExporter {
    fn new(config: MyCustomConfig) -> Self {
        Self { config }
    }
}

impl PushMetricExporter for MyCustomExporter {
    // PushMetricExporter methods...
}
```

### 3. Create a factory function:

```rust
use opentelemetry_config::ConfigurationError;
use opentelemetry_sdk::metrics::MeterProviderBuilder;
use serde_yaml::Value;

fn create_my_custom_exporter(
    mut builder: MeterProviderBuilder,
    config: &Value,
) -> Result<MeterProviderBuilder, ConfigurationError> {
    // Deserialize your custom config
    let custom_config = serde_yaml::from_value::<MyCustomConfig>(config.clone())
        .map_err(|e| ConfigurationError::InvalidConfiguration(e.to_string()))?;

    // Create and configure your exporter
    let exporter = MyCustomExporter::new(custom_config);
    builder = builder.with_periodic_exporter(exporter);

    Ok(builder)
}
```

### 4. Register it with the ConfigurationProvidersRegistry:

```rust
use opentelemetry_config::ConfigurationProvidersRegistry;

let mut registry = ConfigurationProvidersRegistry::new();

registry
    .metrics_mut()
    .register_periodic_exporter_factory(
        "my-custom-exporter".to_string(),
        create_my_custom_exporter,
    );
```

### 5. Use it in your YAML configuration:

```yaml
metrics:
  readers:
    - periodic:
        exporter:
          my-custom-exporter:
            endpoint: "http://localhost:4318"
            timeout: 5000
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
