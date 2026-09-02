# Changelog

## [Unreleased]

### Added
- Forwarded `tls-native` (default) and `tls-rustls` feature flags from `geneva-uploader`. Build with `--no-default-features --features tls-rustls` to use the pure-Rust TLS backend (required for FIPS / OpenSSL-free deployments that install a custom `rustls::crypto::CryptoProvider`).
- Forwarded the opt-in `certificate-auth` feature from `geneva-uploader`.

### Changed
- Bump opentelemetry, opentelemetry_sdk, and opentelemetry-proto versions to 0.32.
- Replace the Git-pinned `otap-df-pdata-views` dependency with the published
  `otel-arrow-dfe-pdata-views` 0.53.0 crate.

## [0.5.0] - 2026-04-13

### Changed
- Bump geneva-uploader version to 0.5.0

## [0.4.0] - 2025-11-12

### Changed
- Bump geneva-uploader version to 0.4.0

## [0.3.0] - 2025-10-17

### Changed
- Bump geneva-uploader version to 0.3.0

## [0.2.0] - 2025-09-24

### Added
- Spans upload functionality

### Changed
- Bump opentelemetry and opentelemetry_sdk versions to 0.31
- Bump opentelemetry-proto version to 0.31

## [0.1.0] - 2025-08-18

### Added
- Initial release of opentelemetry-exporter-geneva
