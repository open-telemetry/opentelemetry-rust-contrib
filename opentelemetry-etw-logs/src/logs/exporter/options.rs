use std::borrow::Cow;

use crate::ProcessorBuildError;

#[derive(Debug)]
pub(crate) struct Options {
    provider_name: Cow<'static, str>,
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
        self.default_event_name()
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
            },
        }
    }

    pub(crate) fn build(self) -> Result<Options, ProcessorBuildError> {
      if let Err(e) = self.validate() {
            return Err(e);
        } 
        Ok(self.options)
    }

    fn validate(&self) -> Result<(), ProcessorBuildError> {
      validate_provider_name(self.options.provider_name.as_ref())?;
      Ok(())
    }
}

fn validate_provider_name(provider_name: &str) -> Result<(), ProcessorBuildError> {
    if provider_name.is_empty() {
        return Err(ProcessorBuildError::EmptyProviderName);
    }
    if provider_name.len() >= 234 {
        return Err(ProcessorBuildError::ProviderNameTooLong);
    }
    if !provider_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ProcessorBuildError::InvalidProviderName);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::common::test_utils;

    #[test]
    fn test_get_event_name_from_default() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = test_utils::new_sdk_log_record();

        let options = Options::builder("test-provider-name").build().unwrap();

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
