# Changelog

## vNext

## v0.3.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.24
- Bump opentelemetry-semantic-conventions version to 0.16

## v0.2.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.23
- Bump opentelemetry-semantic-conventions version to 0.15

### Added

- Add "host.arch" attribute into the `HostResourceDetector`.
- Added `HostResourceDetector` which populates "host.id" attribute. Currently only Linux and macOS are supported.

## v0.1.0

### Added

- Initial Resource detectors implementation
