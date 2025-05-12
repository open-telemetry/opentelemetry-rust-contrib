use opentelemetry::otel_warn;
use std::borrow::Cow;

#[derive(Debug)]
enum ETWEventNameFrom {
    Default,
    Target,
    Name,
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
    pub fn new(provider_name: impl Into<Cow<'static, str>>) -> Self {
        ExporterOptionsBuilder {
            inner: ExporterOptions {
                provider_name: provider_name.into(),
                default_event_name: "Log".into(),
                event_name_from: ETWEventNameFrom::Default,
            },
        }
    }

    /// Sets the default event name to use as fallback if:
    /// - `use_etw_event_name_from_default()` has been selected, or
    /// - it cannot extract name or target from the `SdkLogRecord`.
    pub fn with_default_event_name(
        mut self,
        default_event_name: impl Into<Cow<'static, str>>,
    ) -> Self {
        self.inner.default_event_name = default_event_name.into();
        self
    }

    /// Sets the event name to be the default value ("Log"). The default value may be overridden by `with_default_event_name()`.
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
    pub fn build(self) -> Result<ExporterOptions, String> {
        if let Err(error) = self.validate() {
            otel_warn!(name: "ETW.ExporterOptions.CreationFailed", reason = &error);
            return Err(error);
        }

        Ok(self.inner)
    }

    fn validate(&self) -> Result<(), String> {
        validate_provider_name(&self.inner.provider_name)?;
        Ok(())
    }
}

fn validate_provider_name(provider_name: &str) -> Result<(), String> {
    if provider_name.is_empty() {
        return Err("Provider name cannot be empty.".to_string());
    }
    if provider_name.len() >= 234 {
        return Err("Provider name must be less than 234 characters.".to_string());
    }
    if !provider_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(
            "Provider name must contain only ASCII alphanumeric characters, '_' or '-'."
                .to_string(),
        );
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
    #[should_panic(expected = "Provider name cannot be empty.")]
    fn test_validate_empty_name() {
        let _ = ExporterOptions::builder("").build().unwrap();
    }

    #[test]
    #[should_panic(expected = "Provider name must be less than 234 characters.")]
    fn test_validate_name_longer_than_234_chars() {
        let _ = ExporterOptions::builder("a".repeat(235)).build().unwrap();
    }

    #[test]
    #[should_panic(
        expected = "Provider name must contain only ASCII alphanumeric characters, '_' or '-'."
    )]
    fn test_validate_name_uses_valid_chars() {
        let _ = ExporterOptions::builder("i_have_a_?_").build().unwrap();
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
