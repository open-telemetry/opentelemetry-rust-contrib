# Changelog

## Unreleased

### Added

- New `tls-rustls` feature flag enables a pure-Rust TLS backend (rustls + p12-keystore) as an alternative to the default `tls-native` (native-tls / OpenSSL) backend. The two flags are additive (so `--all-features` builds compile cleanly); if both are enabled simultaneously, `tls-rustls` takes precedence at runtime. Consumers that need FIPS-validated cryptography can install a custom `rustls::crypto::CryptoProvider` (e.g. `rustls-symcrypt`) at process start, and the rustls backend will use it automatically.

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
