# Changelog

## Unreleased

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
