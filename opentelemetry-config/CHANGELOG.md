# Changelog

## vNext

- Align metrics configuration with OpenTelemetry Declarative Configuration schema v1.2.0. YAML now requires `file_format` (for example, `file_format: "1.2"`) and uses `meter_provider` instead of the old `metrics` key.
- Add resource attribute entries with `name`, `value`, and an optional `type`. Supported scalar types are `string` (the default), `bool`, `int`, and `double`; valid null-valued attributes are ignored.
- Add the built-in `console` metric exporter with configurable temporality.
- Change custom factories to receive only their exporter properties and return a `PushMetricExporter`. Reader settings are applied centrally, and previously started readers are shut down if configuration fails.
- Change `register_metric_exporter_factory` to return `Result<(), ConfigurationError>`. Registration rejects the reserved names `console`, `otlp_http`, `otlp_grpc`, and `otlp_file/development`; non-standard names such as `otlp_file` and `prometheus` can be registered as extensions.
- Accept compatible `1.x` file format versions, warning for newer minors. Ignore unknown root properties with a warning while retaining strict nested validation.
- Reject explicit periodic reader `timeout` and zero `interval` values that SDK 0.33 cannot honor. Omitted or null intervals use 60 seconds independently of `OTEL_METRIC_EXPORT_INTERVAL`.
- Reject values containing `${` before creating providers; environment substitution remains unsupported.

## v0.1.0

### Added

- Initial declarative configuration
