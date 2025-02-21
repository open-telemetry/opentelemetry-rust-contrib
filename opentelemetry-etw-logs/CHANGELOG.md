# Changelog

## vNext

- Added the `with_etw_exporter` trait method to `LoggerProviderBuilder`.
  This is now the only way to add an ETW exporter. The following line
  will add an ETW exporter using the given provider name:

  ```rust
  SdkLoggerProvider::builder().with_etw_exporter("my-provider").build();
  ```

- Removed `opentelemetry_etw_logs::{ExporterConfig, ReentrantLogProcessor, ETWExporter}` from the public API. Ability to customize Provider Group, Keyword will be added future.

- Hardcoded `event_enabled` internal method to be true on unit test, improving test coverage.

- Improved test coverage.

## v0.7.0

- Bump msrv to 1.75.0
- Bump opentelemetry and opentelemetry_sdk versions to 0.28

## v0.6.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.27

## v0.5.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.26

## v0.4.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.25

## v0.3.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.24

## v0.2.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.23

## v0.1.0

- Initial Alpha implementation
