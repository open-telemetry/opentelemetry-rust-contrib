# Changelog

## [Unreleased]

### Changed
- Bump pinned `otel-arrow` rev for `otap-df-pdata-views` to `4f522d2e` so
  consumers can unify on a single `otap-df-pdata-views` version and avoid
  duplicate `LogsDataView` trait errors. API-compatible; the view trait
  signatures are unchanged.

## [0.5.0] - 2026-04-13

### Changed
- Bump geneva-uploader version to 0.5.0

## [0.4.0] - 2025-11-12

### Changed
- Bump geneva-uploader version to 0.4.0

## [0.3.0] - 2025-10-17

### Changed
- Bump geneva-uploader version to 0.3.0

## [0.2.0] - 2025-09-24

### Added
- Spans upload functionality

### Changed
- Bump opentelemetry and opentelemetry_sdk versions to 0.31
- Bump opentelemetry-proto version to 0.31

## [0.1.0] - 2025-08-18

### Added
- Initial release of opentelemetry-exporter-geneva
