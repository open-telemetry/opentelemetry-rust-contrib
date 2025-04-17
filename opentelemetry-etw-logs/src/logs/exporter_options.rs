use opentelemetry::otel_warn;
use std::collections::HashMap;

/// TODO: Add documentation
#[non_exhaustive]
#[derive(Debug)]
pub enum EventMapping {
    /// TODO: Add documentation
    HashMap(HashMap<String, String>),
    /// TODO: Add documentation
    FrozenMap(frozen_collections::FzStringMap<String, String>),
}

/// TODO: Add documentation
#[derive(Debug)]
pub struct ExporterOptions {
    provider_name: String,
    event_mapping: Option<EventMapping>,
}

/// TODO: Add documentation
#[derive(Debug)]
pub struct ExporterOptionsBuilder {
    provider_name: String,
    event_mapping: Option<EventMapping>,
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
}

impl ExporterOptionsBuilder {
    pub fn new(provider_name: String) -> Self {
        ExporterOptionsBuilder {
            provider_name,
            event_mapping: None,
        }
    }

    pub fn with_event_mapping(mut self, event_mapping: EventMapping) -> Self {
        self.event_mapping = Some(event_mapping);
        self
    }

    pub fn build(self) -> Result<ExporterOptions, String> {
        if let Err(error) = self.validate() {
            otel_warn!(name: "ETW.ExporterOptions.CreationFailed", reason = &error);
            return Err(error);
        }

        Ok(ExporterOptions {
            provider_name: self.provider_name,
            event_mapping: self.event_mapping,
        })
    }

    fn validate(&self) -> Result<(), String> {
        validate_provider_name(&self.provider_name)?;
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
