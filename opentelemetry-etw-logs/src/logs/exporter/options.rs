use std::borrow::Cow;

use opentelemetry_sdk::logs::SdkLogRecord;

use crate::logs::processor::{ProcessorBuildError, ProcessorBuildErrorKind};

type BoxedEventNameCallback = Box<dyn EventNameCallback>;

#[derive(Debug)]
pub(crate) struct Options {
    provider_name: Cow<'static, str>,
    event_name_callback: Option<BoxedEventNameCallback>,
}

impl Options {
    pub(crate) fn builder(provider_name: impl Into<Cow<'static, str>>) -> OptionsBuilder {
        OptionsBuilder::new(provider_name)
    }

    /// Returns the provider name that will be used for the ETW provider.
    pub(crate) fn provider_name(&self) -> &str {
        &self.provider_name
    }

    /// Returns the default event name that will be used for the ETW events.
    pub(crate) fn default_event_name(&self) -> &str {
        "Log"
    }

    /// Returns the event name to be used for the ETW event given the log record contents and the options.
    pub(crate) fn get_etw_event_name<'a>(
        &'a self,
        log_record: &'a opentelemetry_sdk::logs::SdkLogRecord,
    ) -> &'a str {
        if let Some(callback) = &self.event_name_callback {
            let result = callback(log_record);
            if validate_etw_event_name(result).is_ok() {
                return result;
            }
        }
        self.default_event_name()
    }
}

trait EventNameCallback: Fn(&SdkLogRecord) -> &str + Send + Sync + 'static {}

impl<F> EventNameCallback for F where F: Fn(&SdkLogRecord) -> &str + Send + Sync + 'static {}

impl std::fmt::Debug for dyn EventNameCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ETW event name callback")
    }
}

#[derive(Debug)]
pub(crate) struct OptionsBuilder {
    options: Options,
}

impl OptionsBuilder {
    pub(crate) fn new(provider_name: impl Into<Cow<'static, str>>) -> Self {
        OptionsBuilder {
            options: Options {
                provider_name: provider_name.into(),
                event_name_callback: None,
            },
        }
    }

    pub(crate) fn build(self) -> Result<Options, ProcessorBuildError> {
        self.validate()?;
        Ok(self.options)
    }

    pub(crate) fn etw_event_name_from_callback(
        mut self,
        callback: impl Fn(&SdkLogRecord) -> &str + Send + Sync + 'static,
    ) -> Self {
        self.options.event_name_callback = Some(Box::new(callback));
        self
    }

    fn validate(&self) -> Result<(), ProcessorBuildError> {
        validate_provider_name(self.options.provider_name.as_ref())?;
        Ok(())
    }
}

fn validate_provider_name(provider_name: &str) -> Result<(), ProcessorBuildError> {
    if provider_name.is_empty() {
        return Err(ProcessorBuildError::new(
            ProcessorBuildErrorKind::ProviderNameEmpty,
        ));
    }
    if provider_name.len() >= 234 {
        return Err(ProcessorBuildError::new(
            ProcessorBuildErrorKind::ProviderNameTooLong,
        ));
    }
    if !provider_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ProcessorBuildError::new(
            ProcessorBuildErrorKind::ProviderNameInvalid,
        ));
    }
    Ok(())
}

fn validate_etw_event_name(event_name: &str) -> Result<(), ProcessorBuildError> {
    if event_name.is_empty() {
        return Err(ProcessorBuildError::new(
            ProcessorBuildErrorKind::EventNameEmpty,
        ));
    }
    // TODO: Finish validation for ETW event name
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::common::test_utils;
    use super::*;

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
    fn test_get_event_name_from_callback_returning_const_value() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = test_utils::new_sdk_log_record();

        let options = Options::builder("test-provider-name")
            .etw_event_name_from_callback(|_| "CustomEvent")
            .build()
            .unwrap();

        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "CustomEvent");

        log_record.set_event_name("event-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "CustomEvent");

        log_record.set_target("target-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "CustomEvent");
    }

    #[test]
    fn test_get_event_name_from_callback_returning_name() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = test_utils::new_sdk_log_record();

        let options = Options::builder("test-provider-name")
            .etw_event_name_from_callback(|log_record| log_record.event_name().unwrap_or_default())
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

        log_record.set_target("target-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "event-name");
    }

    #[test]
    fn test_get_event_name_from_callback_returning_target() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = test_utils::new_sdk_log_record();

        let options = Options::builder("test-provider-name")
            .etw_event_name_from_callback(|log_record| {
                log_record.target().map_or("", |target| target.as_ref())
            })
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

        log_record.set_event_name("event-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "target-name");
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
