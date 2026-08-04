# Changelog

## vNext

- A failed ETW event write (`EventBuilder::write` returning a
  nonzero Win32 error) no longer triggers a `debug_assert!` panic. ETW write
  failures are expected, transient runtime conditions (e.g. buffer pressure from
  an active session) that Microsoft documents as diagnostic-only, so the failed
  event is now silently dropped and the exporter continues, instead of
  terminating the process in debug or `debug-assertions`-enabled builds.

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
