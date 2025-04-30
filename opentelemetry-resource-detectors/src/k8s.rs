use opentelemetry::KeyValue;
use opentelemetry_sdk::resource::{Resource, ResourceDetector};
use std::env;
use std::fs::read_to_string;

const K8S_NAMESPACE_PATH: &str = "/var/run/secrets/kubernetes.io/serviceaccount/namespace";

/// Detect Kubernetes information.
///
/// This resource detector returns the following information:
///
/// - Pod name (`k8s.pod.name`)
/// - Namespace (`k8s.namespace.name`).
pub struct K8sResourceDetector;

impl ResourceDetector for K8sResourceDetector {
    fn detect(&self) -> Resource {
        let pod_name = env::var("HOSTNAME").ok();

        let namespace = read_to_string(K8S_NAMESPACE_PATH).ok();

        Resource::builder_empty()
            .with_attributes(
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
                    env::var("NODE_NAME").ok().map(|name| {
                        KeyValue::new(
                            opentelemetry_semantic_conventions::attribute::K8S_NODE_NAME,
                            name,
                        )
                    }),
                ]
                .into_iter()
                .flatten(),
            )
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::{Key, Value};

    #[test]
    fn test_k8s_resource_detector_with_env_vars() {
        temp_env::with_vars(
            [
                ("HOSTNAME", Some("test-pod")),
                ("NODE_NAME", Some("test-node")),
            ],
            || {
                let resource = K8sResourceDetector.detect();

                assert_eq!(resource.len(), 2);

                assert_eq!(
                    resource.get(&Key::from_static_str(
                        opentelemetry_semantic_conventions::attribute::K8S_POD_NAME
                    )),
                    Some(Value::from("test-pod"))
                );

                assert_eq!(
                    resource.get(&Key::from_static_str(
                        opentelemetry_semantic_conventions::attribute::K8S_NODE_NAME
                    )),
                    Some(Value::from("test-node"))
                )
            },
        );
    }

    #[test]
    fn test_k8s_resource_detector_with_missing_env_vars() {
        // make sure no env var is accidentally set
        temp_env::with_vars_unset(["HOSTNAME", "NODE_NAME"], || {
            let resource = K8sResourceDetector.detect();

            assert_eq!(resource.len(), 0);
            assert!(resource
                .get(&Key::from_static_str(
                    opentelemetry_semantic_conventions::attribute::K8S_NODE_NAME
                ))
                .is_none())
        });
    }
}
