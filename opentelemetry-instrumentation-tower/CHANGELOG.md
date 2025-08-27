# Changelog

## vNext

### Changed

* **BREAKING**: Removed `with_meter()` method. The middleware now uses global meter and tracer providers via `opentelemetry::global::meter()` and `opentelemetry::global::tracer()`.
* **BREAKING**: Renamed types. Use the new names:
  - `HTTPMetricsLayer` → `HTTPLayer`
  - `HTTPMetricsService` → `HTTPService`
  - `HTTPMetricsResponseFuture` → `HTTPResponseFuture`
  - `HTTPMetricsLayerBuilder` → `HTTPLayerBuilder`
* Added OpenTelemetry trace support

### Migration Guide

#### API Changes
Before:
```rust
use opentelemetry_instrumentation_tower::HTTPMetricsLayerBuilder;

let layer = HTTPMetricsLayerBuilder::builder()
    .with_meter(meter)
    .build()
    .unwrap();
```

After:
```rust
use opentelemetry_instrumentation_tower::HTTPLayerBuilder;

// Set global providers first
global::set_meter_provider(meter_provider);
global::set_tracer_provider(tracer_provider); // for tracing support

// Then create the layer without explicit meter
let layer = HTTPLayerBuilder::builder()
    .build()
    .unwrap();
```

#### Type Name Changes
- Replace `HTTPMetricsLayerBuilder` with `HTTPLayerBuilder`
- Replace `HTTPMetricsLayer` with `HTTPLayer`
- Replace `HTTPMetricsService` with `HTTPService`
- Replace `HTTPMetricsResponseFuture` with `HTTPResponseFuture`
