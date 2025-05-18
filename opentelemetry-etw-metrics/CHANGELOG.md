# Changelog

## vNext

- Bump tracelogging crate to 1.2.4

## v0.8.0

- Bump opentelemetry and opentelemetry_sdk versions to 0.29
- Bump opentelemetry-proto version to 0.29

## v0.7.1

- Fixed a bug that caused incorrect serialization encoding.
  [#176](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/176)

## v0.7.0

- Bump msrv to 1.75.0
- Bump opentelemetry and opentelemetry_sdk versions to 0.28
- Bump opentelemetry-proto version to 0.28

## v0.6.0

- Bump opentelemetry and opentelemetry_sdk versions to 0.27
- Bump opentelemetry-proto version to 0.27
- Uses internal logging from `opentelemetry` crate, which routes internal logs
  via `tracing`.

## v0.5.0

### Changed

 - Bump opentelemetry and opentelemetry_sdk versions to 0.26
 - Bump opentelemetry-proto version to 0.26
 - Bump rust msrv to v1.71.1

## v0.4.0

- Improved logging when ETW write fails due to size limit being hit.
    [105](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/105)
- Bump opentelemetry,opentelemetry_sdk and opentelemetry-proto versions to 0.25
    [105](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/105)

## v0.3.0

### Changed

 - Bump opentelemetry and opentelemetry_sdk versions to 0.24
 - Bump opentelemetry-proto version to 0.7

## v0.2.0
### Changed

 - Bump opentelemetry and opentelemetry_sdk versions to 0.23
 - Bump opentelemetry-proto version to 0.6

## v0.1.0

### Added

- Initial Alpha implementation
