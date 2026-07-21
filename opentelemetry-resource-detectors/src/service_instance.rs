//! Service instance ID resource detector
//!
//! Generate a unique `service.instance.id`.
use crate::uuid;
use opentelemetry::KeyValue;
use opentelemetry_sdk::resource::ResourceDetector;
use opentelemetry_sdk::Resource;
use std::sync::OnceLock;

/// Detect the `service.instance.id` resource attribute.
///
/// The [OpenTelemetry specification] recommends `service.instance.id` be a
/// random UUID when no inherently unique identifier is available. This detector
/// generates a UUIDv7, which embeds a millisecond-precision timestamp in the
/// first 48 bits followed by random data, so IDs sort chronologically on
/// backends that index by this field.
///
/// The id is generated once and reused for the lifetime of the process, so every
/// resource built from this detector shares the same instance id.
///
/// [OpenTelemetry specification]: https://opentelemetry.io/docs/specs/semconv/resource/service/#service-instance
#[derive(Debug, Default)]
pub struct ServiceInstanceIdResourceDetector;

fn instance_id() -> &'static str {
    static INSTANCE_ID: OnceLock<String> = OnceLock::new();
    INSTANCE_ID.get_or_init(uuid::v7)
}

impl ResourceDetector for ServiceInstanceIdResourceDetector {
    fn detect(&self) -> Resource {
        Resource::builder_empty()
            .with_attributes(vec![KeyValue::new(
                opentelemetry_semantic_conventions::attribute::SERVICE_INSTANCE_ID,
                instance_id(),
            )])
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::ServiceInstanceIdResourceDetector;
    use opentelemetry::{Key, Value};
    use opentelemetry_sdk::resource::ResourceDetector;

    #[test]
    fn detects_service_instance_id() {
        let resource = ServiceInstanceIdResourceDetector.detect();
        assert_eq!(resource.len(), 1);

        let value = resource.get(&Key::from_static_str(
            opentelemetry_semantic_conventions::attribute::SERVICE_INSTANCE_ID,
        ));
        match value {
            Some(Value::String(id)) => assert_eq!(id.as_str().len(), 36),
            other => panic!("expected a string service.instance.id, got {other:?}"),
        }
    }

    #[test]
    fn returns_the_same_id_across_detectors() {
        let key = Key::from_static_str(
            opentelemetry_semantic_conventions::attribute::SERVICE_INSTANCE_ID,
        );
        let first = ServiceInstanceIdResourceDetector.detect().get(&key);
        let second = ServiceInstanceIdResourceDetector.detect().get(&key);
        assert!(first.is_some());
        assert_eq!(first, second);
    }
}
