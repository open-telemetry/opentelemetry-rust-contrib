# Changelog

## vNext

## v0.10.0

- Bump opentelemetry and opentelemetry_sdk versions to 0.29
- Bump opentelemetry-proto version to 0.29

## v0.9.0

- Bump msrv to 1.75.0
- Bump opentelemetry and opentelemetry_sdk versions to 0.28
- Bump opentelemetry-proto version to 0.28

## v0.8.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.27
- Bump opentelemetry-proto version to 0.27
- Uses internal logging from `opentelemetry` crate, which routes internal logs
  via `tracing`
- Add support for skipping the metric data point if the size exceeds 65360 bytes.

## v0.7.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.26,
  opentelemetry-proto version to 0.26.
- Bump msrv to 1.71.1

## v0.6.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.25,
  opentelemetry-proto version to 0.25.

## v0.5.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.24,
  opentelemetry-proto version to 0.7.
- Update prost to 0.13

## v0.4.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.23,
  opentelemetry-proto version to 0.6.
- Bump eventheader version to 0.4.0

## v0.3.0

### Changed

- Bump opentelemetry version to 0.22, opentelemetry_sdk version to 0.22,
  opentelemetry-proto version to 0.5.

## v0.2.2

- Fixed a bug which caused Histogram, Gauge metrics to be dropped.
    [#30](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/30).

## v0.2.1

- Update eventheader version to 0.3.4.
    [#27](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/27).

## v0.2.0

- Fix aggregation selector and temporality so every instruments are aggregated
  correctly with expected delta temporality.
    [#1287](https://github.com/open-telemetry/opentelemetry-rust/pull/1287).

### Changed

- Bump MSRV to 1.65 [#1318](https://github.com/open-telemetry/opentelemetry-rust/pull/1318)
- Include error diagnosing messages for registering tracepoint
    [#1273](https://github.com/open-telemetry/opentelemetry-rust/pull/1273).
- Add version, protocol to schema
    [#1224](https://github.com/open-telemetry/opentelemetry-rust/pull/1224).

## v0.1.0

### Added

- Initial Alpha implementation
