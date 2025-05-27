# Changelog

## vNext

## v0.13.0

### Changed

- Event header name is changed to use "Log" instead of "LogRecord.EventName".
- `ext_dt` and `ext_cloud` structs are flattened.
- **BREAKING**
  - Removed the `with_user_events_exporter` extension method from `LoggerProviderBuilder`.
    - Instead, introduced a builder pattern for configuring the user events exporter, providing greater flexibility.

    **Before:**

    ```rust
    use opentelemetry_sdk::logs::LoggerProviderBuilder;
    use opentelemetry_user_events_logs::UserEventsLoggerProviderBuilderExt;

    let logger_provider = LoggerProviderBuilder::default()
      .with_user_events_exporter("myprovider")
      .build();
    ```

    **After:**

    ```rust
    use opentelemetry_sdk::logs::LoggerProviderBuilder;
    let user_event_processor = opentelemetry_user_events_logs::Processor::builder("myprovider")
        .build()
        .expect("Expected to have processor built");
    LoggerProviderBuilder::default()
        .with_log_processor(user_event_processor)
        .build();
    ```

    The `build()` method of `LoggerProviderBuilder` returns a `Result`, allowing users to handle errors as needed.
    Additional validation: empty provider names are now disallowed.
- Bump opentelemetry-* crates versions to 0.30

## v0.12.0

- Added support for Populating Cloud RoleName, RoleInstance from Resource's
  "service.name" and "service.instance.id" attributes respectively.
- Make exporter reentrant-safe by removing logs that could be bridged back
  to itself.
- Export SeverityNumber from OTel Severity, not EventHeader severity. (They move
  in opposite direction)
- Exporter now unregisters the `Provider` on `shutdown()`.
  [#221](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/221)
- `with_user_event_exporter` method on `LoggerProviderBuilder` renamed to
  `with_user_events_exporter`.

## v0.11.0

- Fixed contention in `event_enabled()` check and `export()` path, by caching the
  EventSets, addressing
  [159](https://github.com/open-telemetry/opentelemetry-rust-contrib/issues/159)
- Added validation for the provider name in `with_user_event_exporter(provider_name)`.
  The provider name must:
  - Be less than 234 characters.
  - Contain only ASCII letters, digits, and the underscore (`'_'`) character.
- Added support for TraceId,SpanId
- Bump opentelemetry and opentelemetry_sdk versions to 0.29

## v0.10.0

- Removed provider group from being appended to the tracepoint name.
  For example, tracepoint `myprovider_L2K1Gmyprovider` becomes `myprovider_L2K1`.
- Added the `with_user_event_exporter` trait method to `LoggerProviderBuilder`.
  This is now the only way to add a user-events exporter. The following line
  will add a user-events exporter using the given provider name:

  ```rust
  SdkLoggerProvider::builder().with_user_event_exporter("my-provider").build();
  ```

- Removed `opentelemetry_user_events_logs::{ExporterConfig, ReentrantLogProcessor, UserEventsExporter}` from the public API. Ability to customize Provider Group, Keyword will be added future.
- `log_record.event_name()` is used to populate EventName. Previous behavior of populating EventName from specially named attributes is removed.
- Fix CommonSchema version to `0x0400` instead of `0x0401`
- Bug fix: `export()` now returns `Err` when the tracepoint is not found.
- Include error number in internal logs, when writing to tracepoint fails.

## v0.9.0

- Bump msrv to 1.75.0
- Bump opentelemetry and opentelemetry_sdk versions to 0.28
- Renamed  `logs_level_enabled` flag to `spec_unstable_logs_enabled` to be consistent with core repo.

## v0.8.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.27

## v0.7.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.26

## v0.6.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.25

## v0.5.0

- **BREAKING** Decouple Exporter creation with the Reentrant processor [#82](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/82)
  The UserEventsExporter is now created separately and passed to the ReentrantProcessor. Update your application code from:
  ```rust
    let reentrant_processor = ReentrantLogProcessor::new("test", None, exporter_config);
  ```
  to:

  ```rust
      let exporter = UserEventsExporter::new("test", None, exporter_config);
      let reentrant_processor = ReentrantLogProcessor::new(exporter);
  ``
- Bump opentelemetry and opentelemetry_sdk versions to 0.24

## v0.4.0

### Changed

- Bump opentelemetry and opentelemetry_sdk versions to 0.23
- Bump eventheader and eventheader_dynamics versions to 0.4

## v0.3.0

### Changed

- Bump opentelemetry version to 0.22, opentelemetry_sdk version to 0.22

## v0.2.0

### Changed

- Bump MSRV to 1.65 [#1318](https://github.com/open-telemetry/opentelemetry-rust/pull/1318)

## v0.1.0

### Added

- Initial Alpha implementation
