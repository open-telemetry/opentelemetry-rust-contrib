use opentelemetry::KeyValue;
use opentelemetry_sdk::resource::{Resource, ResourceDetector};
use std::env;
use std::fs::read_to_string;
use std::time::Duration;

const K8S_NAMESPACE_PATH: &str = "/var/run/secrets/kubernetes.io/serviceaccount/namespace";

/// Detect Kubernetes information.
///
/// This resource detector returns the following information:
///
/// - Pod name (`k8s.pod.name`)
/// - Namespace (`k8s.namespace.name`).
pub struct K8sResourceDetector;

impl ResourceDetector for K8sResourceDetector {
    fn detect(&self, _timeout: Duration) -> Resource {
        let pod_name = env::var("HOSTNAME").ok();

        let namespace = read_to_string(K8S_NAMESPACE_PATH).ok();

        Resource::new(
            [
                pod_name.map(|name| {
                    KeyValue::new(
                        opentelemetry_semantic_conventions::attribute::K8S_POD_NAME,
                        name,
                    )
                }),
                namespace.map(|name| {
                    KeyValue::new(
                        opentelemetry_semantic_conventions::attribute::K8S_NAMESPACE_NAME,
                        name,
                    )
                }),
            ]
            .into_iter()
            .flatten(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::{Key, Value};
    use std::time::Duration;

    #[test]
    fn test_k8s_resource_detector_with_env_vars() {
        temp_env::with_vars([("HOSTNAME", Some("test-pod"))], || {
            let resource = K8sResourceDetector.detect(Duration::from_secs(0));

            assert_eq!(resource.len(), 1);

            assert_eq!(
                resource.get(Key::from_static_str(
                    opentelemetry_semantic_conventions::attribute::K8S_POD_NAME
                )),
                Some(Value::from("test-pod"))
            )
        });
    }

    #[test]
    fn test_k8s_resource_detector_with_missing_env_vars() {
        // make sure no env var is accidentally set
        temp_env::with_vars_unset(["HOSTNAME"], || {
            let resource = K8sResourceDetector.detect(Duration::from_secs(0));

            assert_eq!(resource.len(), 0);
        });
    }
}
