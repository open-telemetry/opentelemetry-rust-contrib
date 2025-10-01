# Changelog

## vNext

## v0.19.0

- Bump opentelemetry and opentelemetry_sdk versions to 0.31
- Bump opentelemetry-http and opentelemetry-semantic-conventions versions to 0.31

## v0.18.0

- Bump opentelemetry and opentelemetry_sdk versions to 0.30
- Bump opentelemetry-http and opentelemetry-semantic-conventions versions to 0.30

## v0.17.0

- `DatadogExporter.export()` doesn't require mutability anymore
- Bump opentelemetry and opentelemetry_sdk versions to 0.29
- Bump opentelemetry-http and opentelemetry-semantic-conventions versions to 0.29

## v0.16.0

- Bump mvrv to 1.75.0
- Bump opentelemetry and opentelemetry_sdk versions to 0.28
- Bump opentelemetry-http and opentelemetry-semantic-conventions versions to 0.28
- Bump thiserror to 2.0
- [Breaking] Replace `opentelemetry::global::shutdown_tracer_provider()` with `tracer_provider.shutdown()` (original PR [opentelemetry-rust#2369](https://github.com/open-telemetry/opentelemetry-rust/pull/2369))
- [Breaking] `DatadogPipelineBuilder::install_simple()` and `DatadogPipelineBuilder::install_batch()` now return `TracerProvider`.
  Additionally, global tracer provider now needs to be set by the user by calling `global::set_tracer_provider(tracer_provider.clone())` (original PR [opentelemetry-rust#1812](https://github.com/open-telemetry/opentelemetry-rust/pull/1812))

## v0.15.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.27
- Bump opentelemetry-http and opentelemetry-semantic-conventions versions to 0.27

## v0.14.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.26
- Bump opentelemetry-http and opentelemetry-semantic-conventions versions to 0.26
- Remove unused itertools dependency

## v0.13.0

### Changed

- Bump opentelemetry and opentelemetry_sdk version to 0.25

## v0.12.0

### Changed

- Bump opentelemetry and opentelemetry_sdk version to 0.24
- Bump hyper to version 1

## v0.10.0

### Added

- Pass DD_GIT_REPOSITORY_URL and DD_GIT_COMMIT_SHA during build

### Changed

- Bump opentelemetry version to 0.22, opentelemetry_sdk version to 0.22

### Changed

- allow send all traces to `datadog-agent` with `agent-sampling` feature.
- allow `datadog-agent` generate metrics from spans for [APM](https://docs.datadoghq.com/tracing/metrics/).

## v0.9.0

### Changed

- Bump MSRV to 1.65 [#1318](https://github.com/open-telemetry/opentelemetry-rust/pull/1318)
- Bump MSRV to 1.64 [#1203](https://github.com/open-telemetry/opentelemetry-rust/pull/1203)

### Fixed

- Do not set an empty span as the active span when the propagator does not find a remote span.
- Change type signature of `with_http_client()` to use the provided generic as argument.

## V0.8.0

### Changed

- Update to opentelemetry-api v0.20.0

### Fixed

- Fix the array encoding length of datadog version 05 exporter #1002

## v0.7.0

### Added
- [Breaking] Add support for unified tagging [#931](https://github.com/open-telemetry/opentelemetry-rust/pull/931).

### Changed
- Update `opentelemetry` to 0.19
- Update `opentelemetry-http` to 0.8
- Update `opentelemetry-semantic-conventions` to 0.11.
- Bump MSRV to 1.57 [#953](https://github.com/open-telemetry/opentelemetry-rust/pull/953)
- Send resource with attributes [#880](https://github.com/open-telemetry/opentelemetry-rust/pull/880).
- Update msgpack accounting for sampling_priority [#903](https://github.com/open-telemetry/opentelemetry-rust/pull/903).
- Update dependencies and bump MSRV to 1.60 [#969](https://github.com/open-telemetry/opentelemetry-rust/pull/969).

## v0.6.0

### Changed

- Allow custom mapping #770
- Update to opentelemetry v0.18.0
- Update to opentelemetry-http v0.7.0
- Update to opentelemetry-semantic-conventions v0.10.0
- Parse config endpoint to remove tailing slash #787
- Add sampling priority tag in spans #792

## v0.5.0

### Changed

- Update to opentelemetry v0.17.0
- Update to opentelemetry-http v0.6.0
- Update to opentelemetry-semantic-conventions v0.9.0

## v0.4.0

### Changed

- Update to opentelemetry v0.16.0

## v0.3.1

### Fixed

- `status_code` must be 0 or 1 #580

## v0.3.0

### Changed

- Update to opentelemetry v0.15.0

## v0.2.0

### Changed

- Disable optional features for reqwest
- Remove default surf features #546
- Update to opentelemetry v0.14.0

## v0.1.0

### Added

- Datadog exporter #446
- Datadog propagator #440

### Changed
- Rename trace config with_default_sampler to with_sampler #482
