//! ETW Exporter implementation for OpenTelemetry spans.
//!
//! Uses TraceLogging Dynamic (TLD) to write span data to ETW
//! following the Microsoft Common Schema v4.0 format.

pub(crate) mod common;
pub(crate) mod options;
mod part_a;
mod part_b;
mod part_c;

pub(crate) use options::Options;

use opentelemetry::Key;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::trace::SpanData;
use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt::Debug;
use std::pin::Pin;
use std::sync::Arc;
use tracelogging_dynamic as tld;

/// Defining 0x01 as default keyword for uncategorized events.
/// Currently used for all events.
const DEFAULT_KEYWORD_UNCATEGORIZED: u64 = 0x01;

// Thread-local EventBuilder to avoid heap allocations on every export.
thread_local! {
    static EVENT_BUILDER: RefCell<tld::EventBuilder> = RefCell::new(tld::EventBuilder::new());
}

#[derive(Default)]
struct Resource {
    pub cloud_role: Option<String>,
    pub cloud_role_instance: Option<String>,
    pub attributes_from_resource: Vec<(Key, opentelemetry::Value)>,
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

/// The ETW exporter for span data.
///
/// Holds a TLD provider and options. Uses thread-local `EventBuilder`
/// for zero-allocation reuse across export calls.
pub(crate) struct ETWExporter {
    provider: Pin<Arc<tld::Provider>>,
    resource: Resource,
    options: Options,
    resource_attribute_keys: Vec<Cow<'static, str>>,
}

impl ETWExporter {
    /// Creates a new ETWExporter, registering the ETW provider.
    pub(crate) fn new(options: Options) -> Self {
        let mut provider_options = tld::Provider::options();
        provider_options.callback(enabled_callback_noop, 0x0);
        let provider = Arc::pin(tld::Provider::new(
            options.provider_name(),
            &provider_options,
        ));

        // Copied from [opentelemetry_etw_logs]:
        // SAFETY: tracelogging (ETW) enables an ETW callback into the provider
        // when `register()` is called. On dynamically created providers, the
        // lifetime of the provider is tied to the object itself, so `unregister()`
        // is called when dropped.
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

    /// Exports a single span to ETW.
    ///
    /// Event write order follows Common Schema: reset → __csver__ → PartA → PartC → PartB → write
    pub(crate) fn export_span_data(&self, span_data: &SpanData) {
        let event_tags: u32 = 0;
        let field_tag: u32 = 0;

        EVENT_BUILDER.with_borrow_mut(|event| {
            event.reset(
                self.options.event_name(),
                tld::Level::Verbose,
                DEFAULT_KEYWORD_UNCATEGORIZED,
                event_tags,
            );

            // Common Schema version as u16 (0x0400 = 1024)
            event.add_u16("__csver__", 1024, tld::OutType::Unsigned, field_tag);

            // Part A: envelope (time, traceId, spanId, cloud role)
            part_a::populate_part_a(event, &self.resource, span_data, field_tag);

            // Part C: custom data (individual typed attributes, resource attributes)
            part_c::populate_part_c(
                event,
                span_data,
                &self.resource,
                self.options.optional_attributes_keys(),
                field_tag,
            );

            // Part B: span payload (_typeName, name, kind, startTime, parentId, links, statusMessage, success)
            part_b::populate_part_b(event, span_data, field_tag);

            // Write event to ETW
            let result = event.write(&self.provider, None, None);

            // event.write() returns 0 for success or a Win32 error from EventWrite for failure.
            // The return value is for diagnostic purposes only and should generally be ignored in retail builds.
            match result {
                0 => (),
                _ => debug_assert!(false, "Failed to write event to ETW. ETW reason: {result}"),
            }
        });
    }

    /// Unregisters the ETW provider.
    pub(crate) fn shutdown(&self) -> OTelSdkResult {
        let res = self.provider.as_ref().unregister();
        if res != 0 {
            return Err(OTelSdkError::InternalFailure(format!(
                "Failed to unregister provider. Win32 error: {res}"
            )));
        }
        Ok(())
    }

    /// Updates the resource identity and optional resource attributes.
    pub(crate) fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
        self.resource.attributes_from_resource.clear();

        for (key, value) in resource.iter() {
            if key.as_str() == "service.name" {
                self.resource.cloud_role = Some(value.to_string());
            } else if key.as_str() == "service.instance.id" {
                self.resource.cloud_role_instance = Some(value.to_string());
            } else if self
                .resource_attribute_keys
                .iter()
                .any(|k| k.as_ref() == key.as_str())
            {
                self.resource
                    .attributes_from_resource
                    .push((key.clone(), value.clone()));
            }
        }
    }
}

impl Debug for ETWExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ETW trace exporter")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug() {
        let exporter = common::test_utils::new_etw_exporter();
        let result = format!("{exporter:?}");
        assert_eq!(result, "ETW trace exporter");
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

    #[test]
    fn test_set_resource() {
        use opentelemetry::KeyValue;

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

        assert_eq!(
            exporter.resource.cloud_role,
            Some("test-service".to_string())
        );
        assert_eq!(
            exporter.resource.cloud_role_instance,
            Some("test-instance".to_string())
        );
        assert_eq!(exporter.resource.attributes_from_resource.len(), 2);
    }
}
