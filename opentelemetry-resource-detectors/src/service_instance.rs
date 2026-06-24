//! Service instance ID resource detector
//!
//! Generate a unique `service.instance.id`.
use crate::uuid;
use opentelemetry::KeyValue;
use opentelemetry_sdk::resource::ResourceDetector;
use opentelemetry_sdk::Resource;

/// Detect the `service.instance.id` resource attribute.
///
/// The [OpenTelemetry specification] recommends `service.instance.id` be a
/// random UUIDv4 when no inherently unique identifier is available.
///
/// [OpenTelemetry specification]: https://opentelemetry.io/docs/specs/semconv/resource/service/#service-instance
#[derive(Debug, Default)]
pub struct ServiceInstanceIdResourceDetector;

impl ResourceDetector for ServiceInstanceIdResourceDetector {
    fn detect(&self) -> Resource {
        Resource::builder_empty()
            .with_attributes(vec![KeyValue::new(
                opentelemetry_semantic_conventions::attribute::SERVICE_INSTANCE_ID,
                uuid::v4(),
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
    fn generates_a_new_id_per_detect() {
        let detector = ServiceInstanceIdResourceDetector;
        let key = Key::from_static_str(
            opentelemetry_semantic_conventions::attribute::SERVICE_INSTANCE_ID,
        );
        assert_ne!(detector.detect().get(&key), detector.detect().get(&key));
    }
}
