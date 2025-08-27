# Changelog

## vNext

### Changed

* **BREAKING**: Removed `with_meter()` method from `HTTPLayerBuilder`. The middleware now uses global providers.
* Added OpenTelemetry trace support

### Migration Guide

Before:
```rust
let layer = HTTPLayerBuilder::builder()
    .with_meter(meter)
    .build()
    .unwrap();
```

After:
```rust
// Set global providers first
global::set_meter_provider(meter_provider);
global::set_tracer_provider(tracer_provider); // for tracing support

// Then create the layer without explicit meter
let layer = HTTPLayerBuilder::builder()
    .build()
    .unwrap();
```
