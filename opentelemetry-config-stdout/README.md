# OpenTelemetry Declarative Configuration for stdout (console)

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

Declarative configuration for applications instrumented with [`OpenTelemetry`]. 

[`OpenTelemetry`]: https://crates.io/crates/opentelemetry

## Overview

This crate provides a declarative configuration extension for OpenTelemetry that enables stdout (console) metric exports. It integrates with the `opentelemetry-config` crate to allow YAML-based configuration of the console exporter.

### Features

- Console/stdout metrics exporter configuration via YAML
- Support for both Delta and Cumulative temporality
- Integration with OpenTelemetry declarative configuration
- Simple registration API for the configurator

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
opentelemetry-config-stdout = "0.1.0"
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
use opentelemetry_config::{ConfiguratorManager, TelemetryConfigurator};
use opentelemetry_config_stdout::ConsolePeriodicExporterConfigurator;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create configurator manager and register stdout exporter
    let mut configurator_manager = ConfiguratorManager::new();
    ConsolePeriodicExporterConfigurator::register_into(&mut configurator_manager);

    // Load configuration from YAML file
    let telemetry_configurator = TelemetryConfigurator::new();
    let providers = telemetry_configurator
        .configure_from_yaml_file(&configurator_manager, "otel-config.yaml")?;

    // Use the configured providers
    if let Some(meter_provider) = providers.meter_provider() {
        // Your application code here
    }

    // Shutdown all providers
    providers.shutdown()?;
    Ok(())
}
```

## Examples

### Console Exporter Example

See the [examples/console](examples/console) directory for a complete working example that demonstrates:

- Setting up a console exporter configurator
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
```

## Contributing

Contributions are welcome! Please feel free to submit issues or pull requests.

## License

This project is licensed under the Apache-2.0 license.

## Release Notes

You can find the release notes (changelog) [here](https://github.com/open-telemetry/opentelemetry-rust-contrib/tree/main/opentelemetry-config-stdout/CHANGELOG.md).
