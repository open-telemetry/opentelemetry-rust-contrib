#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use temp_env;

    #[test]
    fn test_k8s_resource_detector_with_env_vars() {
        // Temporarily set environment variables for the test
        temp_env::with_vars(
            [
                ("K8S_POD_NAME", Some("test-pod")),
                ("K8S_NAMESPACE_NAME", Some("test-namespace")),
                ("K8S_NODE_NAME", Some("test-node")),
            ],
            || {
                // Create the K8sResourceDetector
                let detector = K8sResourceDetector::new();
                // Use the detector to fetch the resources
                let resource = detector.detect(Duration::from_secs(5));
                
                // Assert that the detected resource attributes match the expected values
                assert_eq!(
                    resource,
                    Resource::new(vec![
                        KeyValue::new("k8s.pod.name", "test-pod"),
                        KeyValue::new("k8s.namespace.name", "test-namespace"),
                        KeyValue::new("k8s.node.name", "test-node"),
                    ])
                );
            },
        );
    }

    #[test]
    fn test_k8s_resource_detector_with_missing_env_vars() {
        // Temporarily set only one environment variable to test defaults
        temp_env::with_vars(
            [("K8S_POD_NAME", Some("test-pod"))],
            || {
                // Create the K8sResourceDetector
                let detector = K8sResourceDetector::new();
                // Use the detector to fetch the resources
                let resource = detector.detect(Duration::from_secs(5));

                // Assert that missing values use the default "unknown" values
                assert_eq!(
                    resource,
                    Resource::new(vec![
                        KeyValue::new("k8s.pod.name", "test-pod"),
                        KeyValue::new("k8s.namespace.name", "unknown_namespace"),
                        KeyValue::new("k8s.node.name", "unknown_node"),
                    ])
                );
            },
        );
    }
}
