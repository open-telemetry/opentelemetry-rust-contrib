# Changelog

## vNext

## v0.1.0

Released 2026-Mar-11

- Initial release.
- `Processor` implementing `SpanProcessor` that exports spans to ETW using TraceLogging Dynamic.
- Common Schema v4.0 encoding (Part A / Part B / Part C).
- Builder pattern with provider name validation (cross-compatible and ETW-only modes).
- Configurable event name (defaults to `"Span"`).
- Optional resource attribute promotion to Part C via `with_resource_attributes()`.
- Span attributes exported as individually typed ETW fields.
- Span events and links serialized as JSON.
