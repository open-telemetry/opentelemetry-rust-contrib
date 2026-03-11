use std::borrow::Cow;

/// Configuration options for the ETW trace exporter.
#[derive(Debug)]
pub(crate) struct Options {
    /// Name of the ETW provider to register.
    provider_name: String,

    /// ETW event name for span events.
    /// Defaults to "Span".
    event_name: String,

    /// Resource attribute keys that should be exported with each span.
    /// By default, only `service.name` and `service.instance.id` are exported
    /// as Part A fields (`cloud.role` and `cloud.roleInstance`).
    resource_attribute_keys: Vec<Cow<'static, str>>,

}

impl Options {
    /// Creates a new `Options` with the given provider name.
    pub(crate) fn new(provider_name: impl Into<String>) -> Self {
        Options {
            provider_name: provider_name.into(),
            event_name: "Span".to_string(),
            resource_attribute_keys: Vec::new(),
        }
    }

    /// Returns the provider name.
    pub(crate) fn provider_name(&self) -> &str {
        &self.provider_name
    }

    /// Returns the event name.
    pub(crate) fn event_name(&self) -> &str {
        &self.event_name
    }

    /// Returns the resource attribute keys.
    pub(crate) fn resource_attribute_keys(&self) -> &[Cow<'static, str>] {
        &self.resource_attribute_keys
    }

    /// Sets the event name.
    pub(crate) fn with_event_name(mut self, name: &str) -> Self {
        self.event_name = name.to_string();
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
