# Changelog

### Fixed
- Emit compatible Common Schema metadata for OTLP logs and spans, including `env_ver=4.0`, event-based `env_name`, uppercase `TIMESTAMP`, canonical log severity field casing, and a default Part B log name.
- Include OTLP resource and instrumentation-scope attributes in log payloads. Log-record attributes take precedence over scope attributes, which take precedence over resource attributes, while fixed Common Schema fields remain authoritative.

## [0.7.0] - 2026-09-03

### Changed
- Update `otel-arrow-dfe-pdata` and `otel-arrow-dfe-pdata-views` to 0.54.1.

## [0.6.0] - 2026-09-03

### Added
- New `tls-rustls` feature flag enables a pure-Rust TLS backend as an alternative to the default `tls-native` (native-tls / OpenSSL) backend. The two flags are additive (so `--all-features` builds compile cleanly); if both are enabled simultaneously, `tls-rustls` takes precedence at runtime. No built-in crypto provider (e.g. ring) is compiled in; consumers **must** install a `rustls::crypto::CryptoProvider` (e.g. `rustls-symcrypt`) at process start. The uploader returns a clear error if no provider is found.
- New opt-in `certificate-auth` feature enables PKCS#12 client certificate authentication. With `tls-rustls`, legacy PBES1, RC2, and DES-encrypted keystores are not supported.
- Attribute-based event/table-name routing for logs and spans. `LogsConfig` and `TracesConfig` gained an optional `event_name_mapping` field (`LogsEventNameMapping` / `SpanEventNameMapping`) that routes each record to a destination event/table based on a `routing_key` (logs: event name, resource/scope/log-record attribute; spans: resource/scope/span attribute; plus the reserved `scope.name` / `scope.version` keys) and a source→destination `events` map. Unmapped source values fall back to the configured default event name; entries with an empty destination pass the source value through unchanged. Records/spans are split into one encoded batch per resolved destination. Invalid mappings (empty `events`, blank source keys, or a blank attribute routing-key name) are rejected by `GenevaClient::new`.
- Agent-fed credential source: `GenevaClient::with_agent_fed_source` builds an uploader that pulls a host-provisioned GIG token and routing snapshot (endpoint and account-group-to-moniker map) from an `AgentFedCredentialSource` on each upload, skipping the GCS config-service handshake. New public API: `AgentFedCredentialSource`, `AgentFedCredential`, `AgentFedCredentialFuture`.
- Native multi-moniker routing. `AccountRouting` defines one required default logical account group and optional exact final-event-name overrides. Encoded batches carry their resolved logical group, and uploads resolve that group through the current GCS snapshot to the corresponding primary physical moniker.

### Fixed
- Corrected a misleading startup log message: when `logs.default_event_name` is set without an `event_name_mapping`, `GenevaClient::new` now logs `Configured logs event name routing [default_event_name=...]` instead of `Logs config not initialized; using default values for log events`. The default was always applied correctly; only the log line was wrong. This makes the logs message symmetric with the equivalent spans message.
- Resolve every GCS logical account group to exactly one primary physical
  moniker instead of inspecting physical moniker names for `diag` or depending
  on response ordering. Empty group sets and zero or multiple primaries fail
  explicitly.

### Changed
- Certificate authentication is disabled by default. Enable `certificate-auth` explicitly, or use managed identity, workload identity, or agent-fed authentication.
- Minimize GIG ingestion bearer-token memory remanence by storing resolved
  credentials in zeroizing secret containers and backing sensitive
  `Authorization` headers with application-owned memory that is zeroized when
  released.
- **Breaking:** `GenevaClientConfig.account_group` was replaced by required
  `account_routing: AccountRouting`. The C `GenevaConfig.account_group` is now
  required and `account_group_mapping` supplies optional final-event-name
  overrides.
- Bump opentelemetry-proto version to 0.32.
- Replace the Git-pinned `otap-df-pdata` and `otap-df-pdata-views`
  dependencies with the published `otel-arrow-dfe-pdata` and
  `otel-arrow-dfe-pdata-views` 0.53.0 crates.
- `GenevaClientConfig` now applies signal-specific defaults consistently on emitted batches: when `logs.default_event_name` / `spans.default_event_name` is set, encoded batches use that value as `event_name`; when unset, they fall back to `Log` and `Span` respectively.
- **Breaking:** `GenevaClientConfig.logs` and `.spans` changed from `LogsConfig` / `TracesConfig` to `Option<LogsConfig>` / `Option<TracesConfig>`. Pass `None` to use the default `Log` / `Span` table names.

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
