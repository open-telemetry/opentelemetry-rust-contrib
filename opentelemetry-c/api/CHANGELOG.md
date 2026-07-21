# Changelog

## vNext

### Added

- Initial release of `opentelemetry-c-api` as part of the split of `opentelemetry-c` into
  separate C **API** and **SDK** artifacts. The API library exposes the public trace API
  (tracer providers, tracers, spans) as opaque handles, owns the single process-global
  provider slot with a no-op default, and exposes the internal registration ABI the SDK
  uses to install itself. It depends only on the internal ABI-types crate — never on
  `opentelemetry_sdk`, `opentelemetry-otlp`, or `reqwest` — so instrumentation can link the
  API alone. Existing FFI-safety hardening is preserved (fixed-width discriminants,
  best-effort handle contract, panic firewall, documented thread/lifecycle contracts).
- Criterion benchmark `api_hotpath` measuring the API-only, no-SDK (no-op provider) hot-path
  FFI boundary cost (global provider / tracer acquisition, span start/end, scalar and string
  attribute setters). Run explicitly with `cargo bench -p opentelemetry-c-api`; not a test or
  CI gate. See `opentelemetry-c/README.md` for details.
- Optional header-only convenience helpers over the raw C API (no new ABI symbols, no Rust
  changes): typed `otel_key_value_t` constructors `otel_kv_string` / `otel_kv_bool` /
  `otel_kv_int64` / `otel_kv_double` (`common.h`) and span-status shorthands
  `otel_span_set_ok` / `otel_span_set_error` (`trace.h`). They are `static inline` (guarded for
  C99+/C++ like the existing `otel_cstr`), build POD by value with no allocation/copy, and
  (for the status shorthands) perform exactly the one `otel_span_set_status()` call they wrap.
