
use opentelemetry::otel_warn;
use std::collections::HashMap;

/// TODO: Add documentation
#[non_exhaustive]
#[derive(Debug)]
pub enum EventMapping {
    /// TODO: Add documentation
    HashMap(HashMap<String, String>),
}

/// TODO: Add documentation
#[derive(Debug)]
pub struct ExporterOptions {
    provider_name: String,
    event_mapping: Option<EventMapping>,
    on_missing_key_use_value: bool,
    default_event_name: String,
}

impl ExporterOptions {
    /// TODO: Add documentation
    pub fn builder(provider_name: String) -> ExporterOptionsBuilder {
        ExporterOptionsBuilder::new(provider_name)
    }

    /// TODO: Add documentation
    pub fn provider_name(&self) -> &str {
        &self.provider_name
    }

    /// TODO: Add documentation
    pub fn event_mapping(&self) -> Option<&EventMapping> {
        self.event_mapping.as_ref()
    }

    /// TODO: Add documentation
    pub fn default_event_name(&self) -> String {
        self.default_event_name.clone()
    }

    /// TODO: Add documentation
    pub fn get_etw_event_name(&self, log_record: &opentelemetry_sdk::logs::SdkLogRecord) -> String {
        // Using target for now. This is the default behavior.
        // Future versions of this library may add mechanisms to chose which attribute to use for the mapping key
        if let Some(target) = log_record.target() {
            if let Some(mapping) = self.event_mapping() {
                match mapping {
                    crate::logs::EventMapping::HashMap(map) => {
                      if let Some(name) = map.get(&target.to_string()) {
                        return name.clone();
                      } else if self.on_missing_key_use_value {
                          return target.to_string();
                      }
                      return self.default_event_name();
                    }
                }
            }
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
    pub fn new(provider_name: String) -> Self {
      ExporterOptionsBuilder {
        inner: ExporterOptions {
            provider_name,
            event_mapping: None,
            on_missing_key_use_value: false,
            default_event_name: "Log".to_string(),
        }
      }
    }

    pub fn with_event_mapping(mut self, event_mapping: EventMapping) -> Self {
        self.inner.event_mapping = Some(event_mapping);
        self
    }

    pub fn on_missing_key_use_value(mut self) -> Self {
        self.inner.on_missing_key_use_value = true;
        self
    }

    pub fn on_missing_key_use_default(mut self) -> Self {
        self.inner.on_missing_key_use_value = false;
        self
    }

    pub fn with_default_event_name(mut self, default_event_name: String) -> Self {
        self.inner.default_event_name = default_event_name;
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
        if let Some(ref event_mapping) = self.inner.event_mapping {
            validate_event_mapping(event_mapping)?;
        }
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

fn validate_event_mapping(event_mapping: &EventMapping) -> Result<(), String> {
    // TODO: Review this validation
    match event_mapping {
        EventMapping::HashMap(map) => {
            for (key, value) in map.iter() {
                if key.is_empty() || value.is_empty() {
                    return Err("Event mapping keys and values cannot be empty.".to_string());
                }
            }
        }
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

        let options = ExporterOptions::builder("test_provider_name".to_string())
            .with_default_event_name("default_event_name".into())
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

    #[test]
    fn test_get_event_name_with_mapping() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = test_utils::new_sdk_log_record();

        let mut event_mapping = std::collections::HashMap::new();
        event_mapping.insert("target-name".into(), "event-name".into());

        let options = ExporterOptions::builder("test_provider_name".to_string())
            .with_event_mapping(crate::logs::EventMapping::HashMap(event_mapping))
            .build()
            .unwrap();

        log_record.set_target("target-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "event-name");
    }

    #[test]
    fn test_get_event_name_with_missing_mapping_key_use_default() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = test_utils::new_sdk_log_record();

        let mut event_mapping = std::collections::HashMap::new();
        event_mapping.insert("target-name".into(), "event-name".into());

        let options = ExporterOptions::builder("test_provider_name".to_string())
            .with_event_mapping(crate::logs::EventMapping::HashMap(event_mapping))
            .on_missing_key_use_default()
            .with_default_event_name("default_event_name".into())
            .build()
            .unwrap();

        log_record.set_target("new-missing-target-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "default_event_name");
    }

    #[test]
    fn test_get_event_name_with_missing_mapping_key_use_value() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = test_utils::new_sdk_log_record();

        let mut event_mapping = std::collections::HashMap::new();
        event_mapping.insert("target-name".into(), "event-name".into());

        let options = ExporterOptions::builder("test_provider_name".to_string())
            .with_event_mapping(crate::logs::EventMapping::HashMap(event_mapping))
            .on_missing_key_use_value()
            .with_default_event_name("default_event_name".into())
            .build()
            .unwrap();

        log_record.set_target("new-missing-target-name");
        let result = options.get_etw_event_name(&log_record);
        assert_eq!(result, "new-missing-target-name");
    }
}
