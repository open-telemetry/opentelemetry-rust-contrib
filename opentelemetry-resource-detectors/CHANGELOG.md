# Changelog

## vNext

### Added

- Add "process.runtime.name/process.runtime.version/process.runtime.description" attributes into the `ProcessResourceDetector`.

## v0.8.0

### Changed

- Expose `K8sResourceDetector`, which populates `k8s.pod.name` and `k8s.namespace.name`
- Bump opentelemetry and opentelemetry_sdk versions to 0.29
- Bump opentelemetry-semantic-conventions version to 0.29

## v0.7.0

### Changed

- Bump msrv to 1.75.0
- Bump opentelemetry and opentelemetry_sdk versions to 0.28
- Bump opentelemetry-semantic-conventions version to 0.28

## v0.6.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.27
- Bump opentelemetry-semantic-conventions version to 0.27

## v0.5.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.26
- Bump opentelemetry-semantic-conventions version to 0.26

## v0.4.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.25
- Bump opentelemetry-semantic-conventions version to 0.25

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
