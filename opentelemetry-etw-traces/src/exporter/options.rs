use std::borrow::Cow;

/// Configuration options for the ETW trace exporter.
#[derive(Debug)]
pub(crate) struct Options {
    /// Name of the ETW provider to register.
    provider_name: String,

    /// Default ETW event name for span events.
    /// Defaults to "Span".
    default_event_name: String,

    /// Resource attribute keys that should be exported with each span.
    /// By default, only `service.name` and `service.instance.id` are exported
    /// as Part A fields (`cloud.role` and `cloud.roleInstance`).
    resource_attribute_keys: Vec<Cow<'static, str>>,

    /// Optional list of custom field names to promote as dedicated Part C fields.
    /// If None, all span attributes are promoted. If Some, only matching attributes
    /// are promoted; the rest go into `env_properties` JSON.
    custom_fields: Option<Vec<String>>,
}

impl Options {
    /// Creates a new `Options` with the given provider name.
    pub(crate) fn new(provider_name: impl Into<String>) -> Self {
        Options {
            provider_name: provider_name.into(),
            default_event_name: "Span".to_string(),
            resource_attribute_keys: Vec::new(),
            custom_fields: None,
        }
    }

    /// Returns the provider name.
    pub(crate) fn provider_name(&self) -> &str {
        &self.provider_name
    }

    /// Returns the default event name.
    pub(crate) fn default_event_name(&self) -> &str {
        &self.default_event_name
    }

    /// Returns the resource attribute keys.
    pub(crate) fn resource_attribute_keys(&self) -> &Vec<Cow<'static, str>> {
        &self.resource_attribute_keys
    }

    /// Returns the custom fields set, if configured.
    pub(crate) fn custom_fields(&self) -> Option<&Vec<String>> {
        self.custom_fields.as_ref()
    }

    /// Sets the default event name.
    pub(crate) fn with_event_name(mut self, name: &str) -> Self {
        self.default_event_name = name.to_string();
        self
    }

    /// Sets the resource attributes to export.
    pub(crate) fn with_resource_attributes<I, S>(mut self, attributes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Cow<'static, str>>,
    {
        self.resource_attribute_keys = attributes.into_iter().map(|s| s.into()).collect();
        self
    }
}
