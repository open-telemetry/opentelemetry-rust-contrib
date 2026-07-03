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
