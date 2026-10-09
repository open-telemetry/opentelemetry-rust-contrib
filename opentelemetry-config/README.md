# OpenTelemetry Declarative Configuration

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

| Status        |           |
| ------------- |-----------|
| Stability     | alpha     |
| Supported Schema | [OpenTelemetry Configuration Schema v1.2.0](https://github.com/open-telemetry/opentelemetry-configuration/releases/tag/v1.2.0) |

Declarative configuration for applications instrumented with [`OpenTelemetry`].

[`OpenTelemetry`]: https://crates.io/crates/opentelemetry

## Overview

This crate provides a declarative, YAML-based configuration approach for the OpenTelemetry Rust SDK. Instead of programmatically building telemetry providers with code, you can define your OpenTelemetry configuration in YAML files and load them at runtime.

The configuration model targets the official [OpenTelemetry Configuration Schema v1.2.0](https://github.com/open-telemetry/opentelemetry-configuration/releases/tag/v1.2.0).

> **Note**: This milestone implements a working, schema-aligned subset of declarative `MeterProvider` configuration. It does **not** claim full schema compliance. Unsupported or unimplemented schema fields are validated and explicitly rejected with informative errors rather than silently ignored.

### Implemented Subset (v1.2.0)

- **Root schema**:
  - `file_format: "1.2"` (required)
- **Resource attributes**:
  - `resource.attributes`: list of `{ name, value, type? }` entries
  - Explicit scalar attribute types: `string` (default), `bool`, `int` (64-bit integer), and `double` (64-bit float)
  - `null` attribute values are ignored according to schema semantics
- **MeterProvider**:
  - `meter_provider.readers`: list of one or more `periodic` readers
  - `periodic.interval`: optional non-negative integer in milliseconds (schema default: 60,000 ms)
  - Built-in standard `console` exporter (`console: {}`) with optional `temporality_preference` (`cumulative`, `delta`, or `low_memory`)
  - Custom periodic exporter factories registered via `ConfigurationProviderRegistry`
- **End-to-end telemetry lifecycle**:
  - Deserialization → Validation → `SdkMeterProvider` construction → Metric recording → Export → `force_flush` → `shutdown`

## Quick Start

### 1. Create a Schema-Aligned YAML File

Create `otel-config.yaml`:

```yaml
file_format: "1.2"

resource:
  attributes:
    - name: service.name
      value: "my-service"
    - name: service.version
      value: "1.0.0"
    - name: deployment.environment
      value: "production"
    - name: instance.id
      value: 1
      type: int

meter_provider:
  readers:
    - periodic:
        interval: 60000  # milliseconds (optional, default: 60000)
        exporter:
          console:
            temporality_preference: cumulative
```

### 2. Configure, Record Metrics, Flush, and Shutdown

```rust
use std::error::Error;
use opentelemetry::{metrics::MeterProvider as _, KeyValue};
use opentelemetry_config::{
    providers::TelemetryProviders,
    ConfigurationProviderRegistry,
};

fn main() -> Result<(), Box<dyn Error>> {
    let registry = ConfigurationProviderRegistry::default();
    let yaml_str = std::fs::read_to_string("otel-config.yaml")?;

    // 1. Configure telemetry providers
    let providers = TelemetryProviders::configure_from_yaml_str(&registry, &yaml_str)?;

    // 2. Use the configured MeterProvider
    if let Some(meter_provider) = providers.meter_provider() {
        let meter = meter_provider.meter("my_app");
        let requests_counter = meter.u64_counter("requests_total").build();

        // 3. Record metrics
        requests_counter.add(1, &[KeyValue::new("route", "/users")]);

        // 4. Flush metrics to exporters
        meter_provider.force_flush()?;

        // 5. Clean shutdown
        meter_provider.shutdown()?;
    }

    Ok(())
}
```

### 3. Run the Built-In Console Example

You can run the included console example directly:

```bash
cargo run -p opentelemetry-config --example console
```

## Custom Periodic Exporters

External or experimental periodic exporters can be plugged into the declarative configuration using the provider registry:

```rust
use opentelemetry_config::{ConfigurationProviderRegistry, ConfigurationError};
use opentelemetry_sdk::metrics::MeterProviderBuilder;

let mut registry = ConfigurationProviderRegistry::default();

registry.register_metric_exporter_factory("my_custom_exporter", |builder: MeterProviderBuilder, periodic_yaml: &str| {
    // Deserialize custom configuration from periodic_yaml and attach reader to builder
    Ok(builder)
});
```

See [examples/custom](examples/custom) for a full runnable custom exporter example.

## Validation & Error Handling

To avoid silent configuration drift or unexpected runtime behavior, the crate performs strict validation:

| Rule | Behavior |
|---|---|
| Missing `file_format` | Rejected with `InvalidConfiguration("Missing required field 'file_format'")` |
| Unsupported `file_format` | Any value other than `"1.2"` is rejected |
| Old `metrics` key | Rejected with an informative message instructing to use `meter_provider` |
| Empty or missing `readers` | Rejected; `meter_provider.readers` must contain at least one reader |
| Reader cardinality | Each reader must contain exactly one variant (`periodic` or `pull`) |
| Pull readers | Rejected with `InvalidConfiguration` (pull readers deferred to future milestones) |
| Exporter cardinality | Each periodic reader must specify exactly one exporter in `exporter` |
| Periodic reader `timeout` | **Rejected**. See [Known SDK Limitations](#known-sdk-limitations) |
| Standard unsupported exporters | `otlp_http`, `otlp_grpc`, `prometheus`, etc. return `UnsupportedExporter` |
| Unregistered custom exporter | Returns `NotRegisteredProvider` indicating missing registration |
| Resource attributes | Attribute name must not be empty; declared type must match the value. Null-valued entries are ignored; repeated names use the last non-null value. |
| Unsupported resource features | `resource.detection`, `resource.schema_url`, `resource.attributes_list` are rejected if supplied |
| Unsupported MeterProvider features | `views`, `exemplar_filter`, `meter_configurator`, `view_matching_mode` are rejected if supplied |

## Known SDK Limitations

- **Synchronous Periodic Reader Timeout**: The upstream OpenTelemetry configuration schema specifies an optional `timeout` on periodic readers. However, the synchronous `PeriodicReader` in OpenTelemetry Rust SDK 0.33 does not enforce collection or export timeouts. Rather than silently ignoring this option and giving a false expectation of timeout enforcement, explicitly specifying `timeout` returns an `InvalidConfiguration` error in this milestone.
- **Interval = 0**: The schema permits an `interval` of 0 ms. In the OpenTelemetry Rust SDK synchronous reader, passing an interval of 0 causes the SDK to revert to its default interval of 60 seconds.

## Deferred Features (Future Milestones)

The following features defined in the v1.2.0 schema are intentionally deferred to subsequent PRs:

1. **OTLP Exporters**: OTLP HTTP and OTLP gRPC metric exporters
2. **Prometheus & Pull Readers**: Pull metric readers and Prometheus exporters
3. **Views & Aggregations**: Metric views, custom stream aggregations, and bucket boundaries
4. **Exemplars & Cardinality**: Exemplar filter configurations and cardinality limits
5. **Richer Resource Configuration**: Resource detectors, schema URL, and comma-separated attribute lists
6. **Other Telemetry Signals**: `tracer_provider`, `logger_provider`, and context propagators

## License

This project is licensed under the Apache-2.0 license.
