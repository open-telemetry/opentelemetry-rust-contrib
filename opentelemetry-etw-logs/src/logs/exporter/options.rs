use opentelemetry_sdk::logs::SdkLogRecord;
use std::borrow::Cow;
use std::error::Error;

type BoxedEventNameCallback = Box<dyn EventNameCallback>;

#[derive(Debug)]
pub(crate) struct Options {
    provider_name: Cow<'static, str>,
    event_name_callback: Option<BoxedEventNameCallback>,
}

impl Options {
    pub(crate) fn new(provider_name: impl Into<Cow<'static, str>>) -> Options {
        Options {
            provider_name: provider_name.into(),
            event_name_callback: None,
        }
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

    pub(crate) fn etw_event_name_from_callback(
        mut self,
        callback: impl Fn(&SdkLogRecord) -> &str + Send + Sync + 'static,
    ) -> Self {
        self.event_name_callback = Some(Box::new(callback));
        self
    }
}

trait EventNameCallback: Fn(&SdkLogRecord) -> &str + Send + Sync + 'static {}

impl<F> EventNameCallback for F where F: Fn(&SdkLogRecord) -> &str + Send + Sync + 'static {}

impl std::fmt::Debug for dyn EventNameCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ETW event name callback")
    }
}

fn validate_etw_event_name(event_name: &str) -> Result<(), Box<dyn Error>> {
    if event_name.is_empty() {
        return Err("Event name cannot be empty.".into());
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

        let options =
            Options::new("test-provider-name").etw_event_name_from_callback(|_| "CustomEvent");

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

        let options = Options::new("test-provider-name")
            .etw_event_name_from_callback(|log_record| log_record.event_name().unwrap_or_default());

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

        let options =
            Options::new("test-provider-name").etw_event_name_from_callback(|log_record| {
                log_record.target().map_or("", |target| target.as_ref())
            });

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
}
