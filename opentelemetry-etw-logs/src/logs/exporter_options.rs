use opentelemetry::otel_warn;
use std::borrow::Cow;

/// TODO: Add documentation
#[derive(Debug)]
pub struct ExporterOptions {
    provider_name: Cow<'static, str>,
    default_event_name: Cow<'static, str>,
}

impl ExporterOptions {
    /// TODO: Add documentation
    pub fn builder(provider_name: impl Into<Cow<'static, str>>) -> ExporterOptionsBuilder {
        ExporterOptionsBuilder::new(provider_name)
    }

    /// TODO: Add documentation
    pub(crate) fn provider_name(&self) -> &str {
        &self.provider_name
    }

    /// TODO: Add documentation
    pub(crate) fn default_event_name(&self) -> &str {
        self.default_event_name.as_ref()
    }

    /// TODO: Add documentation
    pub(crate) fn get_etw_event_name<'a>(
        &'a self,
        log_record: &'a opentelemetry_sdk::logs::SdkLogRecord,
    ) -> &'a str {
        // Using target for now. This is the default behavior.
        // Future versions of this library may add mechanisms to chose which attribute to use for the mapping key
        if let Some(target) = log_record.target() {
            return target.as_ref();
        }
        self.default_event_name()
    }
}

/// TODO: Add documentation
#[derive(Debug)]
pub struct ExporterOptionsBuilder {
    inner: ExporterOptions,
}

impl ExporterOptionsBuilder {
    pub fn new(provider_name: impl Into<Cow<'static, str>>) -> Self {
        ExporterOptionsBuilder {
            inner: ExporterOptions {
                provider_name: provider_name.into(),
                default_event_name: "Log".into(),
            },
        }
    }

    pub fn with_default_event_name(
        mut self,
        default_event_name: impl Into<Cow<'static, str>>,
    ) -> Self {
        self.inner.default_event_name = default_event_name.into();
        self
    }

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
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err("Provider name must contain only ASCII letters, digits, and '_'.".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::exporter::common::test_utils;

    #[test]
    fn test_get_default_event_name() {
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
    fn test_get_event_name_with_provided_default_event_name() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = test_utils::new_sdk_log_record();

        let options = ExporterOptions::builder("test_provider_name")
            .with_default_event_name("default_event_name")
            .build()
            .unwrap();

        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "default_event_name");

        log_record.set_event_name("event-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "default_event_name");

        log_record.set_target("target-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "default_event_name");
    }
}
