# Changelog

## [Unreleased]

### Added
- New `tls-rustls` feature flag enables a pure-Rust TLS backend (rustls + p12-keystore) as an alternative to the default `tls-native` (native-tls / OpenSSL) backend. The two flags are additive (so `--all-features` builds compile cleanly); if both are enabled simultaneously, `tls-rustls` takes precedence at runtime. No built-in crypto provider (e.g. ring) is compiled in; consumers **must** install a `rustls::crypto::CryptoProvider` (e.g. `rustls-symcrypt`) at process start. The uploader returns a clear error if no provider is found.
- Agent-fed credential source: `GenevaClient::with_agent_fed_source` builds an uploader that pulls a host-provisioned GIG token and routing (endpoint, moniker) from an `AgentFedCredentialSource` on each upload, skipping the GCS config-service handshake. New public API: `AgentFedCredentialSource`, `AgentFedCredential`, `AgentFedCredentialFuture`.

### Changed
- Bump opentelemetry-proto version to 0.32.
- Bump pinned `otel-arrow` rev for `otap-df-pdata` and `otap-df-pdata-views`
  to `4f522d2e` so consumers can unify on a single `otap-df-pdata-views`
  version and avoid duplicate `LogsDataView` trait errors. API-compatible;
  the view trait signatures are unchanged.
- `GenevaClientConfig` now applies signal-specific defaults consistently on emitted batches: when `logs.default_event_name` / `spans.default_event_name` is set, encoded batches use that value as `event_name`; when unset, they fall back to `Log` and `Span` respectively.

## [0.5.0] - 2026-04-13

### Changed

- `GenevaClient::upload_batch` now returns `Result<(), UploadError>` instead of `Result<(), String>`. The new `UploadError` enum exposes the HTTP status code, parsed `Retry-After` duration, and error category so callers can implement retry strategies without string parsing.
- Replaced `md5` crate with RustCrypto `md-5` crate
- Bump version to 0.5.0

## [0.4.0] - 2025-11-12

### Changed
- Updated `azure_core` dependency from 0.27.0 to 0.29.0
- Updated `azure_identity` dependency from 0.27.0 to 0.29.0

## [0.3.0] - 2025-10-17

### Changed
- Minor internal updates

## [0.2.0] - 2025-09-24

### Added
- HTTP/1.1 upload support with keep-alive connections
- Support for Span upload

### Changed
- Bump opentelemetry-proto version to 0.31

## [0.1.0] - 2025-08-18

### Added
- Initial release of geneva-uploader
