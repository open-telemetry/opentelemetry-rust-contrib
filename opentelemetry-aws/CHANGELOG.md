# Changelog

## vNext

## v0.17.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.29.0
- Breaking change in the way XrayIdGenerator is configured:
  
  ```rust
  // Before
  SdkTracerProvider::builder()
      .with_config(trace::config().with_id_generator(XrayIdGenerator::default()))
      .build();
  
  // After
  SdkTracerProvider::builder()
      .with_id_generator(XrayIdGenerator::default())
      .build();
  ```

## v0.16.0

- Bump msrv to 1.75.0
- Bump opentelemetry and opentelemetry_sdk versions to 0.27.0

## v0.15.0

- Bump opentelemetry and opentelemetry_sdk versions to 0.27.0

## v0.14.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.26.0

## v0.13.0

### Added

- `LambdaResourceDetector` has been added to the crate to detect AWS Lambda attributes. To enable it in your code, use the feature `detector-aws-lambda`.

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.25.0

## v0.12.0

- Bump opentelemetry and opentelemetry_sdk versions to 0.24.0
- Update hyper to 1.4.1

## v0.11.0

### Changed

-  Bump opentelemetry and opentelemetry_sdk versions to 0.23.0

## v0.10.0

### Changed

- Move Xray IdGenerator from `opentelemetry-rust` to `opentelemetry-aws` [#33](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/33)
- Bump opentelemetry version to 0.22.0, opentelemetry_sdk version to 0.22.0

## v0.9.0

### Changed

- Update to opentelemetry v0.21.0
- Bump MSRV to 1.65 [#1318](https://github.com/open-telemetry/opentelemetry-rust/pull/1318)
- Bump MSRV to 1.64 [#1203](https://github.com/open-telemetry/opentelemetry-rust/pull/1203)

## v0.8.0

### Changed

- Update to opentelemetry v0.20.0

## v0.7.0

### Added

- Add public functions for AWS trace header [#887](https://github.com/open-telemetry/opentelemetry-rust/pull/887).

### Changed

- Bump MSRV to 1.57 [#953](https://github.com/open-telemetry/opentelemetry-rust/pull/953)
- Update dependencies and bump MSRV to 1.60 [#969](https://github.com/open-telemetry/opentelemetry-rust/pull/969).

## v0.6.0

### Changed

- reduce `tokio` feature requirements #750
- Update to opentelemetry v0.18.0

### Fixed

- Fix XrayPropagator when no header is present #867

## v0.5.0

### Changed

- Update to opentelemetry v0.17.0

## v0.4.0

### Changed

- Update to opentelemetry v0.16.0

## v0.3.0

### Changed

- Update to opentelemetry v0.15.0

## v0.2.0

### Changed

- Update to opentelemetry v0.14.0

## v0.1.0

### Added

- AWS XRay propagator #446
