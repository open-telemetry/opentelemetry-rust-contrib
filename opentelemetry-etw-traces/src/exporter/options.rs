use std::borrow::Cow;

/// Configuration options for the ETW trace exporter.
#[derive(Debug)]
pub(crate) struct Options {
    /// Name of the ETW provider to register.
    provider_name: String,

    /// ETW event name for each span.
    /// Defaults to "Span".
    event_name: String,

    /// Emit the flat Geneva DGrep mapping instead of the TraceLogging mapping.
    geneva_mapping: bool,

    /// Resource attribute keys that should be exported with each span.
    /// By default, only `service.name` and `service.instance.id` are exported
    /// as Part A fields (`ext_cloud { role, roleInstance }`).
    resource_attribute_keys: Vec<Cow<'static, str>>,
}

impl Options {
    /// Creates a new `Options` with the given provider name.
    pub(crate) fn new(provider_name: impl Into<String>) -> Self {
        Options {
            provider_name: provider_name.into(),
            event_name: "Span".to_string(),
            geneva_mapping: false,
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

    pub(crate) fn geneva_mapping(&self) -> bool {
        self.geneva_mapping
    }

    pub(crate) fn with_geneva_mapping(mut self, enabled: bool) -> Self {
        self.geneva_mapping = enabled;
        self
    }

    /// Returns the resource attribute keys.
    pub(crate) fn resource_attribute_keys(&self) -> &[Cow<'static, str>] {
        &self.resource_attribute_keys
    }

    /// Sets the ETW EventName that the exporter writes for every exported span.
    ///
    /// This is a single, exporter-wide value applied to all spans; there is no
    /// per-span override.
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
