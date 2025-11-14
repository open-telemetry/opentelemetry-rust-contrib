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
- Simple registration API for declarative configuration

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
use opentelemetry_config::{ConfigurationProvidersRegistry, providers::TelemetryProvider};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create configuration registry and register console exporter
    let mut registry = ConfigurationProvidersRegistry::new();
    let metrics_registry = registry.metrics_mut();
    metrics_registry.register_periodic_exporter_factory(
        "console".to_string(),
        opentelemetry_config_stdout::register_console_exporter
    );

    // Load configuration from YAML file
    let telemetry_provider = TelemetryProvider::new();
    let providers = telemetry_provider
        .configure_from_yaml_file(&registry, "otel-config.yaml")?;

    // Use the configured providers
    if let Some(meter_provider) = providers.meter_provider() {
        // Your application code here

        // Shutdown the created meter provider when done.
        meter_provider.shutdown()?;
    }

    Ok(())
}
```

## Examples

### Console Exporter Example

See the [examples/console](examples/console) directory for a complete working example that demonstrates:

- Setting up a console exporter factory
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

#### Configuration Options

- **`temporality`** (optional): Controls how metrics are aggregated
  - `delta`: Reports the change since the last export (useful for rate-based metrics like requests per second)
  - `cumulative`: Reports the total accumulated value since the start (default, useful for gauges and cumulative counters)

## Contributing

Contributions are welcome! Please feel free to submit issues or pull requests.

## License

This project is licensed under the Apache-2.0 license.

## Release Notes

You can find the release notes (changelog) [here](CHANGELOG.md).
