# Changelog

## vNext

- Added `ProcessorBuilder::disable_cross_compat_provider_name()` method that allows the processor to use a provider name that is not compatible with UserEvents, but conforming with ETW requirements which allows hyphens (`-`).

## v0.9.0

Released 2025-Jun-19

- Added validation to provider name.
- Added optional feature `serde_json` to serialize List and Maps.
- **BREAKING**
  - Removed the `with_etw_exporter` extension method from `LoggerProviderBuilder`.
    - Instead, introduced a builder pattern for configuring the ETW exporter, providing greater flexibility.

    **Before:**

    ```rust
    use opentelemetry_etw_logs::ETWLoggerProviderBuilderExt;
    use opentelemetry_sdk::logs::SdkLoggerProvider;

    let logger_provider = SdkLoggerProvider::builder()
      .with_etw_exporter("provider-name")
      .build();
    ```

    **After:**

    ```rust
    use opentelemetry_etw_logs::Processor;
    use opentelemetry_sdk::logs::SdkLoggerProvider;

    let processor = Processor::builder("provider-name")
      .build()
      .expect("Valid provider name is required to build an ETW Processor.");
    SdkLoggerProvider::builder()
      .with_log_processor(processor)
      .build();
    ```

- Bump tracelogging crate to 1.2.4
- Bump opentelemetry and opentelemetry_sdk versions to 0.30

## v0.8.0

- Added the `with_etw_exporter` trait method to `LoggerProviderBuilder`.
  This is now the only way to add an ETW exporter. The following line
  will add an ETW exporter using the given provider name:

  ```rust
  SdkLoggerProvider::builder().with_etw_exporter("provider-name").build();
  ```

  Event name now will be inferred from the `LogRecord` being emitted. If no name is given, it defaults to `Log`.
- Removed `opentelemetry_etw_logs::{ExporterConfig, ReentrantLogProcessor, ETWExporter}` from the public API. Ability to customize Provider Group or Keyword may be added in the future.
- Renamed `logs_level_enabled` feature to `spec_unstable_logs_enabled` to match `opentelemetry` features.
- `default` feature does not enable `spec_unstable_logs_enabled` anymore.
- Bump opentelemetry and opentelemetry_sdk versions to 0.29
- Added support for TraceId,SpanId
- Added support for populating cloud `role` and `roleInstance` from Resource's `service.name` and `service.instance.id` attributes respectively.
- `_typeName` field uses "Log" instead of "Logs".
- Exporter now unregisters the Etw provider on `shutdown()`.
  [#222](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/222)

## v0.7.0

- Bump msrv to 1.75.0
- Bump opentelemetry and opentelemetry_sdk versions to 0.28

## v0.6.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.27

## v0.5.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.26

## v0.4.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.25

## v0.3.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.24

## v0.2.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.23

## v0.1.0

- Initial Alpha implementation
