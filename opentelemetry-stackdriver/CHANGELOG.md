# Changelog

## vNext

- Update `tonic` dependency version to 0.13

## v0.26.0

- Update gRPC schemas

### Changed

- Added support for `MonitoredResource::CloudFunction`, `MonitoredResource::AppEngine`,
  `MonitoredResource::ComputeEngine`, and `MonitoredResource::KubernetesEngine`
- Update to opentelemetry v0.29.0, opentelemetry_sdk v0.29.0, opentelemetry-semantic-conventions v0.29.0
- Drop `futures-core` from dependencies

## v0.25.0

- Bump msrv to 1.75.0
- Update to opentelemetry v0.28.0, opentelemetry_sdk v0.28.0, opentelemetry-semantic-conventions v0.28.0
- Remove `server` feature from tonic dependency
- Bump thiserror to 2.0

## v0.24.0

### Changed

- Update to opentelemetry v0.27.0, opentelemetry_sdk v0.27.0, opentelemetry-semantic-conventions v0.27.0

## v0.23.0

### Changed

- Update to opentelemetry v0.26.0, opentelemetry_sdk v0.26.0, opentelemetry-semantic-conventions v0.26.0

## v0.22.0

### Changed

- Update to opentelemetry v0.25.0, opentelemetry_sdk v0.25.0, opentelemetry-semantic-conventions v0.25.0
- Added support for `MonitoredResource::CloudRunJob` [#100](https://github.com/open-telemetry/opentelemetry-rust-contrib/issues/100)

## v0.21.0

### Changed

- Update to opentelemetry v0.24.0 [#92](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/92)
- Remove `yup-authorizer` feature, which seems to be unused; the yup-oath2 dependency does not seem get much maintenance
  [#92](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/92)
- Bump http to 1 and reqwest to 0.12 [#92](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/92)
- Bump prost (0.13), tonic-build (0.12) and tonic (0.12)
  [#92](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/92)

## v0.20.0

### Changed

- Update to opentelemetry v0.23.0 [#69](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/69)
- Bump gcp_auth to 0.12 [#75](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/75)
- Bump yup-oauth2 to 9 [#71](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/71)
- Bump hyper-rustls to 0.25 [#58](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/58)

## v0.19.1

### Fixed

- Fixed Cargo features for `GcpAuthorizer` [#51](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/51)

## v0.19.0

### Added

- Added support for `GoogleTraceContextPropagator` [#25](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/25)

### Changed

- Use gcp_auth as the default authorizer [#50](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/50)
  yup-oauth2 is still supported and can be enabled via the `yup-authorizer` feature.
- Bump opentelemetry version to 0.22, opentelemetry_sdk version to 0.22 [#39](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/39)
- Bump gcp_auth to 0.11 [#50](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/50)

## v0.18.0

### Changed

- Bump MSRV to 1.65 [#1318](https://github.com/open-telemetry/opentelemetry-rust/pull/1318)
- Bump MSRV to 1.64 [#1203](https://github.com/open-telemetry/opentelemetry-rust/pull/1203)

## v0.17.0

### Added

- Send resource along with span attributes and kind/status #1035
- Add option to authenticate with existing GCP Authentication Manager #1128

### Changed

- Update gRPC schemas #992
- Upgrade gcp-auth to 0.9 #1137
- Update to opentelemetry v0.20.0

## v0.16.0

### Changed
- Update to `opentelemetry` v0.19.
- Update to `opentelemetry-semantic-conventions` v0.11.
- Bump MSRV to 1.57 [#953](https://github.com/open-telemetry/opentelemetry-rust/pull/953).
- Update dependencies and bump MSRV to 1.60 [#969](https://github.com/open-telemetry/opentelemetry-rust/pull/969).
- Update grpc schemas [#992](https://github.com/open-telemetry/opentelemetry-rust/pull/992).

## v0.15.0

### Added

- Added mappings from OTel attributes to Google Cloud Traces #744
- Added `MonitoredResource::CloudRunRevision` #847

### Changed

- Upgrade to opentelemetry v0.18.0
- Upgrade to opentelemetry-semantic-conventions v0.10
- update tonic and prost #825

### Fixed

- Fix `LogEntry.trace` not populated correctly #850

## v0.14.0

### Changed

- Upgrade to new gcp_auth version (#722)
- Stop leaking dependency error types into public API (#722)
- Clarify type of MonitoredResource (#722)

### Fixed

- Fixed issue with futures dependency (#722)
- Don't set up logging channel if no logging is configured (#722)

## v0.13.0

### Changed

- Send export errors to global error handler (#705)
- Return `impl Future` to avoid spawning inside library (#703)
- Implement builder API to simplify configuration (#702)
- Use TLS configuration provided by tonic (#702)
- Optionally send events to Cloud Logging (#702)
- Exclude default `tonic-build` features #635
- Update `gcp_auth` dependency to `0.5.0` #639
- Include the server's message in error display #642
- Update `tonic` to 0.6 #660
- Update gcp_auth and yup-oauth2 to latest versions #700
- Update to opentelemetry v0.17.0

### Fixed

- Avoid calling log from inside exporter #709

## v0.12.0

### Changed

- Update to opentelemetry v0.16.0

## v0.11.0

### Changed

- Update to opentelemetry v0.15.0

## v0.10.0

### Changed

- Update to opentelemetry v0.14.0

## v0.9.0

### Changed
- Move opentelemetry-stackdriver into opentelemetry-rust repo #487
