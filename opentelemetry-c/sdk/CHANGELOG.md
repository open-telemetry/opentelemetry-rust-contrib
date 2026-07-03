# Changelog

## vNext

### Added

- Initial release of `opentelemetry-c-sdk` as part of the split of `opentelemetry-c` into
  separate C **API** and **SDK** artifacts. The SDK library provides the OTLP HTTP/protobuf
  exporter, batch span processor, and `otel_sdk_*` lifecycle behind the C ABI. Installing as
  global (or fetching a provider handle) registers the SDK's implementation into the API
  cdylib's global provider slot across the C ABI, so API-only instrumentation observes it.
  Selectable TLS backend (`native-tls` default, or `rustls-tls`); bounded C-provided batch
  sizes; panic-safe entry points; local parent/child span semantics; force-flush and
  shutdown. A `cross_artifact` integration test proves API-only spans export through the SDK
  across the separate cdylibs.
- Pipeline object model: the SDK setup is decomposed into a generic trace exporter
  (`otel_trace_exporter_t`) and span processor (`otel_span_processor_t`), built by the OTLP
  trace exporter builder (`otlp_trace_exporter.h`) and batch span processor builder
  (`batch_span_processor.h`), and assembled through `otel_sdk_builder_add_span_processor`.
  OTLP/batch-specific setters were removed from the SDK builder. Builders transfer ownership
  of their children on `OTEL_STATUS_OK`. The generic exporter/processor handles are opaque
  extension points for future exporter/processor kinds without an ABI break.
- Criterion benchmark `sdk_hotpath` measuring the SDK-backed hot path (tracer acquisition
  through the installed global provider, span start/end, attribute setters, and a bounded
  event) with a real OTLP-exporter + batch-processor pipeline. It runs with no collector and
  no network export (the exporter targets a closed loopback port; flushes fail fast and are
  discarded), is not an export/throughput benchmark, and is not a CI gate. Run explicitly with
  `cargo bench -p opentelemetry-c-sdk`. See `opentelemetry-c/README.md` for details.
- `otel_otlp_trace_exporter_builder_add_header` now rejects a duplicate header key with
  `OTEL_STATUS_INVALID_ARGUMENT` (and a `otel_last_error_message()` diagnostic) instead of
  silently overwriting the previously added value.
