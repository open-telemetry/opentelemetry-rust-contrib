use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt::Debug;
use std::pin::Pin;
use std::sync::Arc;

use tracelogging_dynamic as tld;

use opentelemetry::logs::Severity;
use opentelemetry::{logs::AnyValue, otel_debug, Key, Value};
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};

pub(crate) mod common;
pub(crate) mod options;
mod part_a;
mod part_b;
mod part_c;

pub(crate) use options::Options;

// Thread-local EventBuilder to avoid heap allocations on every export.
thread_local! {
    static EVENT_BUILDER: RefCell<tld::EventBuilder> = RefCell::new(tld::EventBuilder::new());
}

#[derive(Default)]
struct Resource {
    pub cloud_role: Option<String>,
    pub cloud_role_instance: Option<String>,
    pub attributes_from_resource: Vec<(Key, AnyValue)>,
}

pub(crate) struct ETWExporter {
    provider: Pin<Arc<tld::Provider>>,
    resource: Resource,
    options: Options,
    resource_attribute_keys: HashSet<Cow<'static, str>>,
}

fn enabled_callback_noop(
    _source_id: &tld::Guid,
    _event_control_code: u32,
    _level: tld::Level,
    _match_any_keyword: u64,
    _match_all_keyword: u64,
    _filter_data: usize,
    _callback_context: usize,
) {
    // Unused callback.
}

impl ETWExporter {
    const KEYWORD: u64 = 1;

    pub(crate) fn new(options: Options) -> Self {
        let mut provider_options = tld::Provider::options();

        provider_options.callback(enabled_callback_noop, 0x0);
        let provider = Arc::pin(tld::Provider::new(
            options.provider_name(),
            &provider_options,
        ));
        // SAFETY: tracelogging (ETW) enables an ETW callback into the provider when `register()` is called.
        // This might crash if the provider is dropped without calling unregister before.
        // This only affects static providers.
        // On dynamically created providers, the lifetime of the provider is tied to the object itself, so `unregister()` is called when dropped.
        unsafe {
            provider.as_ref().register();
        }

        let resource_attribute_keys = options.resource_attribute_keys().clone();

        ETWExporter {
            provider,
            resource: Default::default(),
            resource_attribute_keys,
            options,
        }
    }

    fn enabled(&self, level: tld::Level) -> bool {
        // On unit tests, we skip this check to be able to test the exporter as no provider is active.
        if cfg!(test) {
            return true;
        }

        self.provider.enabled(level, Self::KEYWORD)
    }

    pub(crate) fn export_log_data(
        &self,
        log_record: &opentelemetry_sdk::logs::SdkLogRecord,
        _instrumentation: &opentelemetry::InstrumentationScope,
    ) {
        // TODO: If severity_number is not set, then fail the export rather than assuming Debug.
        let otel_level = log_record.severity_number().unwrap_or(Severity::Debug);
        let level = common::convert_severity_to_level(otel_level);

        if !self.enabled(level) {
            return;
        };

        let event_tags: u32 = 0; // TBD name and event_tag values
        let field_tag: u32 = 0;

        EVENT_BUILDER.with_borrow_mut(|event| {
            // reset
            event.reset(
                self.options.get_etw_event_name(log_record),
                level,
                Self::KEYWORD,
                event_tags,
            );

            event.add_u16("__csver__", 1024, tld::OutType::Unsigned, field_tag); // 0x400 hex

            part_a::populate_part_a(event, &self.resource, log_record, field_tag);

            let event_id = part_c::populate_part_c(event, log_record, &self.resource, field_tag);

            part_b::populate_part_b(event, log_record, otel_level, event_id);

            // Write event to ETW
            let result = event.write(&self.provider, None, None);

            // event.write() above returns 0 for success or a Win32 error from EventWrite for failure.
            // The return value is for diagnostic purposes only and should generally be ignored in retail builds.
            match result {
                0 => (),
                _ => debug_assert!(false, "Failed to write event to ETW. ETW reason: {result}"),
            }
        })
    }

    pub(crate) fn shutdown(&self) -> OTelSdkResult {
        let res = self.provider.as_ref().unregister();
        if res != 0 {
            return Err(OTelSdkError::InternalFailure(format!(
                "Failed to unregister provider. Win32 error: {res}"
            )));
        }
        Ok(())
    }

    #[cfg(feature = "spec_unstable_logs_enabled")]
    pub(crate) fn event_enabled(
        &self,
        level: Severity,
        _target: &str,
        _name: Option<&str>,
    ) -> bool {
        self.enabled(common::convert_severity_to_level(level))
    }

    pub(crate) fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
        // Clear previous resource attributes
        self.resource.attributes_from_resource.clear();

        // Process resource attributes
        for (key, value) in resource.iter() {
            // Special handling for cloud role and instance
            // as they are used in PartA of the Common Schema format.
            if key.as_str() == "service.name" {
                self.resource.cloud_role = Some(value.to_string());
            } else if key.as_str() == "service.instance.id" {
                self.resource.cloud_role_instance = Some(value.to_string());
            } else if self.resource_attribute_keys.contains(key.as_str()) {
                self.resource
                    .attributes_from_resource
                    .push((key.clone(), val_to_any_value(value)));
            } else {
                // Other attributes are ignored
                otel_debug!(name: "UserEvents.ResourceAttributeIgnored", key = key.as_str(), message = "To include this attribute, add it via with_resource_attributes() method in the processor builder.");
            }
        }
    }
}

impl Debug for ETWExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ETW log exporter")
    }
}

fn val_to_any_value(val: &Value) -> AnyValue {
    match val {
        Value::Bool(b) => AnyValue::Boolean(*b),
        Value::I64(i) => AnyValue::Int(*i),
        Value::F64(f) => AnyValue::Double(*f),
        Value::String(s) => AnyValue::String(s.clone()),
        _ => AnyValue::String("".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_log_data() {
        let record = common::test_utils::new_sdk_log_record();
        let exporter = common::test_utils::new_etw_exporter();
        let instrumentation = common::test_utils::new_instrumentation_scope();

        exporter.export_log_data(&record, &instrumentation);
    }

    #[test]
    fn test_event_name() {
        use opentelemetry::logs::LogRecord;

        let mut log_record = common::test_utils::new_sdk_log_record();

        log_record.set_event_name("event-name");

        let exporter = common::test_utils::new_etw_exporter();
        let instrumentation = common::test_utils::new_instrumentation_scope();
        exporter.export_log_data(&log_record, &instrumentation);
    }

    #[test]
    fn test_event_resources() {
        use opentelemetry::logs::LogRecord;
        use opentelemetry::KeyValue;

        let mut log_record = common::test_utils::new_sdk_log_record();

        log_record.set_event_name("event-name");

        let mut exporter = common::test_utils::new_etw_exporter();
        exporter.set_resource(
            &opentelemetry_sdk::Resource::builder()
                .with_attributes([
                    KeyValue::new("service.name", "cloud-role-name"),
                    KeyValue::new("service.instance.id", "cloud-role-instance"),
                ])
                .build(),
        );
        let instrumentation = common::test_utils::new_instrumentation_scope();
        exporter.export_log_data(&log_record, &instrumentation);
    }

    #[test]
    fn test_event_resources_with_custom_attributes() {
        use opentelemetry::logs::LogRecord;
        use opentelemetry::KeyValue;

        let mut log_record = common::test_utils::new_sdk_log_record();
        log_record.set_event_name("event-name");

        // Create exporter with custom resource attributes
        let options = Options::new("test_provider")
            .with_resource_attributes(vec!["custom_attribute1", "custom_attribute2"]);

        let mut exporter = ETWExporter::new(options);

        exporter.set_resource(
            &opentelemetry_sdk::Resource::builder()
                .with_attributes([
                    KeyValue::new("service.name", "test-service"),
                    KeyValue::new("service.instance.id", "test-instance"),
                    KeyValue::new("custom_attribute1", "value1"),
                    KeyValue::new("custom_attribute2", "value2"),
                    KeyValue::new("custom_attribute3", "value3"), // This should be ignored
                ])
                .build(),
        );

        // Verify that only the configured attributes are stored
        assert_eq!(
            exporter.resource.cloud_role,
            Some("test-service".to_string())
        );
        assert_eq!(
            exporter.resource.cloud_role_instance,
            Some("test-instance".to_string())
        );
        assert_eq!(exporter.resource.attributes_from_resource.len(), 2);

        // Check that the correct attributes are stored
        let attrs: std::collections::HashMap<String, String> = exporter
            .resource
            .attributes_from_resource
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), format!("{:?}", v)))
            .collect();
        assert!(attrs.contains_key("custom_attribute1"));
        assert!(attrs.contains_key("custom_attribute2"));
        assert!(!attrs.contains_key("custom_attribute3"));

        let instrumentation = common::test_utils::new_instrumentation_scope();
        exporter.export_log_data(&log_record, &instrumentation);
    }

    #[test]
    fn test_debug() {
        let exporter = common::test_utils::new_etw_exporter();
        let result = format!("{exporter:?}");
        assert_eq!(result, "ETW log exporter");
    }

    #[test]
    fn test_callback_noop() {
        enabled_callback_noop(
            &tld::Guid::from_name("provider-name"),
            0,
            tld::Level::Verbose,
            0,
            0,
            0,
            0,
        );
    }
}
