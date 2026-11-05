# Changelog

## vNext

- Add the opt-in `experimental-geneva-mapping` layout from upstream #874.
- Apply stack-backed ID/timestamp formatting, borrowed error descriptions, and
  reduced attribute searches to both layouts, including `env_time`.
- Benchmark serialization and enabled export for nested and flat Geneva output.
- Add idiomatic SDK-only recording/context/cleanup and directly measured end-to-end
  lifecycle benchmarks for both layouts.
- Bump OpenTelemetry dependencies to 0.33.1.

## v0.2.0

Released 2026-Sep-21

- Bump OpenTelemetry dependencies to 0.33.

## v0.1.1
- Do not panic on debug builds when transient errors are returned by `EventBuilder::write`.
  [#737](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/737)


## v0.1.0

Released 2026-Mar-11

- Initial release.
- `Processor` implementing `SpanProcessor` that exports spans to ETW using TraceLogging Dynamic.
- Common Schema v4.0 encoding (Part A / Part B / Part C).
- Builder pattern with provider name validation (cross-compatible and ETW-only modes).
- Configurable event name (defaults to `"Span"`).
- Optional resource attribute promotion to Part C via `with_resource_attributes()`.
- Span attributes exported as individually typed ETW fields.
- Span links serialized as JSON when the optional `serde_json` feature is enabled.
