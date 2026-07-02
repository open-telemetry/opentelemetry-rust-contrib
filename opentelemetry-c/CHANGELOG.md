# Changelog

## vNext

### Added

- Initial release of `opentelemetry-c`: a Rust-backed C API and SDK for OpenTelemetry
  traces.
  - Opaque-handle C ABI with panic-safe entry points and explicit status codes.
    Caller-supplied discriminants (span kind, span status code, attribute type,
    boolean) cross the ABI as fixed-width `uint32_t` values and are range-validated,
    so an out-of-range value can never construct an invalid Rust enum.
  - SDK builder with resource attributes, OTLP HTTP/protobuf exporter (blocking
    client, no user-managed async runtime), and batch span processor options.
  - Selectable TLS backend for HTTPS export: `native-tls` (default) or `rustls-tls`.
  - Tracer provider / tracer / span API: global provider installation, span creation
    with parent linking, typed attributes (string/bool/int64/double), events, status,
    rename, and end.
  - Force flush and shutdown with timeouts.
  - C headers (`common.h`, `trace.h`, `sdk.h`, `api.h`), a `c-basic-traces` example
    with a Makefile, Rust unit + FFI integration tests, and a best-effort C header
    compile test.
