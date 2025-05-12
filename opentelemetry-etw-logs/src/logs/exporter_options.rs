use opentelemetry::otel_warn;
use std::borrow::Cow;
use thiserror::Error;

#[derive(Debug)]
enum ETWEventNameFrom {
    Default,
    Target,
    Name,
}


#[derive(Error, Debug, PartialEq)]
/// Errors that can occur while building the `ExporterOptions`.
#[non_exhaustive]
pub enum ExporterOptionsBuildError {
    #[error("Provider name cannot be empty.")]
    EmptyProviderName,
    #[error("Provider name must be less than 234 characters.")]
    ProviderNameTooLong,
    #[error("Provider name must contain only ASCII alphanumeric characters, '_' or '-'.")]
    InvalidProviderName,
}

/// Options used by the Exporter.
#[derive(Debug)]
pub struct ExporterOptions {
    provider_name: Cow<'static, str>,
    default_event_name: Cow<'static, str>,
    event_name_from: ETWEventNameFrom,
}

impl ExporterOptions {
    /// Creates a new instance of `ExporterOptionsBuilder` with the given provider name.
    ///
    /// By default, all events will be exported to the "Log" ETW event. See `ExporterOptionsBuilder` docs for details on how to override this behavior.
    pub fn builder(provider_name: impl Into<Cow<'static, str>>) -> ExporterOptionsBuilder {
        ExporterOptionsBuilder::new(provider_name)
    }

    /// Returns the provider name that will be used for the ETW provider.
    pub(crate) fn provider_name(&self) -> &str {
        &self.provider_name
    }

    /// Returns the default event name that will be used for the ETW events.
    pub(crate) fn default_event_name(&self) -> &str {
        self.default_event_name.as_ref()
    }

    /// Returns the event name to be used for the ETW event given the log record contents and the options.
    pub(crate) fn get_etw_event_name<'a>(
        &'a self,
        log_record: &'a opentelemetry_sdk::logs::SdkLogRecord,
    ) -> &'a str {
        // Using target for now. This is the default behavior.
        match self.event_name_from {
            ETWEventNameFrom::Default => return self.default_event_name(),
            ETWEventNameFrom::Target => {
                if let Some(target) = log_record.target() {
                    return target.as_ref();
                }
            }
            ETWEventNameFrom::Name => {
                if let Some(name) = log_record.event_name() {
                    return name;
                }
            }
        }
        self.default_event_name()
    }
}

/// Builder for `ExporterOptions`.
#[derive(Debug)]
pub struct ExporterOptionsBuilder {
    inner: ExporterOptions,
}

impl ExporterOptionsBuilder {
    /// Creates a new instance of `ExporterOptionsBuilder` with the given provider name.
    ///
    /// By default, all events will be exported to the "Log" ETW event, as if user has called:
    /// - `builder.with_default_event_name("Log")`
    /// - `builder.use_etw_event_name_from_default()`
    pub fn new(provider_name: impl Into<Cow<'static, str>>) -> Self {
        ExporterOptionsBuilder {
            inner: ExporterOptions {
                provider_name: provider_name.into(),
                default_event_name: "Log".into(),
                event_name_from: ETWEventNameFrom::Default,
            },
        }
    }

    /// Sets a default event name different than "Log" to be used as fallback if:
    /// - `use_etw_event_name_from_default()` has been selected, or
    /// - it cannot extract name or target from the `SdkLogRecord` when `use_etw_event_name_from_default()` or `use_etw_event_name_from_target()` are selected.
    pub fn with_default_event_name(
        mut self,
        default_event_name: impl Into<Cow<'static, str>>,
    ) -> Self {
        self.inner.default_event_name = default_event_name.into();
        self
    }

    /// Sets the event name to always be the default value ("Log"). The default value may be overridden by `with_default_event_name()`.
    ///
    /// This is the default behavior.
    pub fn use_etw_event_name_from_default(mut self) -> Self {
        self.inner.event_name_from = ETWEventNameFrom::Default;
        self
    }

    /// Sets the event name to be the `target` from `SdkLogRecord`. If `target` is `None`, it uses the default value ("Log"). The default value may be overridden by `with_default_event_name()`.
    pub fn use_etw_event_name_from_target(mut self) -> Self {
        self.inner.event_name_from = ETWEventNameFrom::Target;
        self
    }

    /// Sets the event name to be the `name` from `SdkLogRecord`. If `name` is `None`, it uses the default value ("Log"). The default value may be overridden by `with_default_event_name()`.
    pub fn use_etw_event_name_from_name(mut self) -> Self {
        self.inner.event_name_from = ETWEventNameFrom::Name;
        self
    }

    /// Validates the options given by consuming itself and returning the `ExporterOptions` or an error.
    pub fn build(self) -> Result<ExporterOptions, ExporterOptionsBuildError> {
        if let Err(error) = self.validate() {
            otel_warn!(name: "ETW.ExporterOptions.CreationFailed", reason = &error.to_string());
            return Err(error);
        }

        Ok(self.inner)
    }

    fn validate(&self) -> Result<(), ExporterOptionsBuildError> {
        validate_provider_name(&self.inner.provider_name)?;
        Ok(())
    }
}

fn validate_provider_name(provider_name: &str) -> Result<(), ExporterOptionsBuildError> {
    if provider_name.is_empty() {
        return Err(ExporterOptionsBuildError::EmptyProviderName);
    }
    if provider_name.len() >= 234 {
        return Err(ExporterOptionsBuildError::ProviderNameTooLong);
    }
    if !provider_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ExporterOptionsBuildError::InvalidProviderName);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::exporter::common::test_utils;

    #[test]
    fn test_get_event_name() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = test_utils::new_sdk_log_record();

        let options = test_utils::test_options();

        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "Log");

        log_record.set_event_name("event-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "Log");

        log_record.set_target("target-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "Log");
    }

    #[test]
    fn test_get_event_name_from_default() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = test_utils::new_sdk_log_record();

        let options = ExporterOptions::builder("test-provider-name")
            .use_etw_event_name_from_default()
            .build()
            .unwrap();

        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "Log");

        log_record.set_event_name("event-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "Log");

        log_record.set_target("target-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "Log");
    }

    #[test]
    fn test_get_event_name_from_target() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = test_utils::new_sdk_log_record();

        let options = ExporterOptions::builder("test-provider-name")
            .use_etw_event_name_from_target()
            .build()
            .unwrap();

        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "Log");

        log_record.set_event_name("event-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "Log");

        log_record.set_target("target-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "target-name");
    }

    #[test]
    fn test_get_event_name_from_name() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = test_utils::new_sdk_log_record();

        let options = ExporterOptions::builder("test-provider-name")
            .use_etw_event_name_from_name()
            .build()
            .unwrap();

        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "Log");

        log_record.set_target("target-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "Log");

        log_record.set_event_name("event-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "event-name");
    }

    #[test]
    fn test_get_event_name_with_provided_default_event_name() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = test_utils::new_sdk_log_record();

        let options = ExporterOptions::builder("test-provider-name")
            .with_default_event_name("default-event-name")
            .build()
            .unwrap();

        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "default-event-name");

        log_record.set_event_name("event-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "default-event-name");

        log_record.set_target("target-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "default-event-name");
    }

    #[test]
    fn test_get_event_name_from_name_with_provided_default_event_name() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = test_utils::new_sdk_log_record();

        let options = ExporterOptions::builder("test-provider-name")
            .with_default_event_name("default-event-name")
            .use_etw_event_name_from_name()
            .build()
            .unwrap();

        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "default-event-name");

        log_record.set_target("target-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "default-event-name");

        log_record.set_event_name("event-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "event-name");
    }

    #[test]
    fn test_get_event_name_from_target_with_provided_default_event_name() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = test_utils::new_sdk_log_record();

        let options = ExporterOptions::builder("test-provider-name")
            .with_default_event_name("default-event-name")
            .use_etw_event_name_from_target()
            .build()
            .unwrap();

        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "default-event-name");

        log_record.set_event_name("event-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "default-event-name");

        log_record.set_target("target-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "target-name");
    }

    #[test]
    fn test_validate_empty_name() {
        assert_eq!(ExporterOptions::builder("").build().unwrap_err(), ExporterOptionsBuildError::EmptyProviderName);
    }

    #[test]
    fn test_validate_name_longer_than_234_chars() {
        assert_eq!(ExporterOptions::builder("a".repeat(235)).build().unwrap_err(), ExporterOptionsBuildError::ProviderNameTooLong);
    }

    #[test]
    fn test_validate_name_uses_valid_chars() {
        assert_eq!(ExporterOptions::builder("i_have_a_?_").build().unwrap_err(), ExporterOptionsBuildError::InvalidProviderName);
    }

    #[test]
    fn test_validate_provider_name() {
        let result = validate_provider_name("valid_provider_name");
        assert!(result.is_ok());

        let result = validate_provider_name("");
        assert!(result.is_err());

        let result = validate_provider_name("a".repeat(235).as_str());
        assert!(result.is_err());

        let result = validate_provider_name("i_have_a_-_");
        assert!(result.is_ok());

        let result = validate_provider_name("_?_");
        assert!(result.is_err());

        let result = validate_provider_name("abcdefghijklmnopqrstuvwxyz");
        assert!(result.is_ok());

        let result = validate_provider_name("ABCDEFGHIJKLMNOPQRSTUVWXYZ");
        assert!(result.is_ok());

        let result = validate_provider_name("1234567890");
        assert!(result.is_ok());

        let result = validate_provider_name(
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890_",
        );
        assert!(result.is_ok());
    }
}
