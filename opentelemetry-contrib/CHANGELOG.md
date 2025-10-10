# Changelog

## vNext

## v0.23.0

- Bump opentelemetry and opentelemetry_sdk versions to 0.31

## v0.22.0

- Bump opentelemetry and opentelemetry_sdk versions to 0.30

## v0.21.0

- Bump `base64` version to 0.22
- `rt-async-std` feature removed, as it is removed from upstream `opentelemetry_sdk`
- Bump opentelemetry and opentelemetry_sdk versions to 0.29
- Bump opentelemetry-semantic-conventions version to 0.29

## v0.20.0

- Bump msrv to 1.75.0
- Bump opentelemetry and opentelemetry_sdk versions to 0.28
- Bump opentelemetry-semantic-conventions version to 0.28
- [Breaking] `JaegerJsonExporter::install_batch()` now returns `TracerProvider`.
  Additionally, global tracer provider now needs to be set by the user by calling `global::set_tracer_provider(tracer_provider.clone())` (original PR [opentelemetry-rust#1812](https://github.com/open-telemetry/opentelemetry-rust/pull/1812))

## v0.19.0

- Bump opentelemetry and opentelemetry_sdk versions to 0.27
- Bump opentelemetry-semantic-conventions version to 0.27

## v0.18.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.26
- Bump opentelemetry-semantic-conventions version to 0.26

## v0.17.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.25
- Bump opentelemetry-semantic-conventions version to 0.25

## v0.16.0 

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.24.0
- Bump opentelemetry-semantic-conventions version to 0.16

## v0.15.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.23
- Bump opentelemetry-semantic-conventions version to 0.15

## v0.14.0

### Changed

- Update `BinaryFormat::deserialize_from_bytes` to take a byte slice instead of a Vec [#32](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/32)
- Bump opentelemetry version to 0.22, opentelemetry_sdk version to 0.22

## v0.13.0

### Changed

- Bump MSRV to 1.65 [#1318](https://github.com/open-telemetry/opentelemetry-rust/pull/1318)
- Bump MSRV to 1.64 [#1203](https://github.com/open-telemetry/opentelemetry-rust/pull/1203)

## v0.12.0

### Added

-  Implement w3c trace context response propagation #998

### Changed

- update to opentelemetry-api v0.20.0

## v0.11.0

### Changed
- Handle `parent_span_id` in jaeger JSON exporter [#907](https://github.com/open-telemetry/opentelemetry-rust/pull/907).
- Bump MSRV to 1.57 [#953](https://github.com/open-telemetry/opentelemetry-rust/pull/953).
- Update dependencies and bump MSRV to 1.60 [#969](https://github.com/open-telemetry/opentelemetry-rust/pull/969).
- Implement w3c trace context response propagation [#998](https://github.com/open-telemetry/opentelemetry-rust/pull/998).

## v0.10.0

### Added

- Add jaeger JSON file exporter #814

### Changed

- Rename binary propagator's functions #776
- Update to opentelemetry v0.18.0

## v0.9.0

### Changed

- Update to opentelemetry v0.17.0

## v0.8.0

### Changed

- Update to opentelemetry v0.16.0

## v0.7.0

### Changed

- Update to opentelemetry v0.15.0

## v0.6.0

### Changed

- Update to opentelemetry v0.14.0

## v0.5.0

### Removed
- Moved aws related function to `opentelemetry-aws` crate. #446
- Moved datadog related function to `opentelemetry-datadog` crate. #446

### Changed

- Update to opentelemetry v0.13.0

## v0.4.0

### Changed

- Update to opentelemetry v0.12.0
- Support tokio v1.0 #421
- Use opentelemetry-http for http integration #415

## v0.3.0

### Changed

- Update to opentelemetry v0.11.0

## v0.2.0

### Changed

- Update to opentelemetry v0.10.0
- Move binary propagator and base64 format to this crate #343

## v0.1.0

### Added

- Datadog exporter
