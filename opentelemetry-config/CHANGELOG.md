# Changelog

## vNext

- Align metrics configuration with OpenTelemetry Declarative Configuration schema v1.2.0: require `file_format`, replace the old `metrics` YAML shape with `meter_provider`, add typed resource attributes, and support the built-in console exporter.
- Change custom factories to return exporters, apply reader settings centrally, and shut down readers after configuration failures.
- Accept compatible file format versions and explicitly reject unsupported environment references.
- Ignore unknown root properties with a warning while keeping nested configuration validation strict.
- Reject zero periodic intervals that the SDK cannot honor, and allow non-standard exporter names to reach registered factories.
- Return a `Result` from exporter registration and reject reserved standard push exporter names.

## v0.1.0

### Added

- Initial declarative configuration
