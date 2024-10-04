# Changelog

## vNext

## v0.6.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.25

## v0.5.0

- **BREAKING** Decouple Exporter creation with the Reentrant processor [#82](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/82)
  The UserEventsExporter is now created separately and passed to the ReentrantProcessor. Update your application code from:
  ```rust
    let reenterant_processor = ReentrantLogProcessor::new("test", None, exporter_config);
  ```
  to:

  ```rust
      let exporter = UserEventsExporter::new("test", None, exporter_config);
      let reenterant_processor = ReentrantLogProcessor::new(exporter);
  ``
- Bump opentelemetry and opentelemetry_sdk versions to 0.24

## v0.4.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.23
- Bump eventheader and eventheader_dynamics versions to 0.4

## v0.3.0

### Changed

- Bump opentelemetry version to 0.22, opentelemetry_sdk version to 0.22

## v0.2.0

### Changed

- Bump MSRV to 1.65 [#1318](https://github.com/open-telemetry/opentelemetry-rust/pull/1318)

## v0.1.0

### Added

- Initial Alpha implementation
