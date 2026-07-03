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
