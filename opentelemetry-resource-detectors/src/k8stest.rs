#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_k8s_resource_detector_with_env_vars() {
        // Set environment variables for testing
        env::set_var("K8S_POD_NAME", "test-pod");
        env::set_var("K8S_NAMESPACE_NAME", "test-namespace");
        env::set_var("K8S_NODE_NAME", "test-node");

        // Create an instance of K8sResourceDetector
        let detector = K8sResourceDetector;
        
        // Call detect function to get the Resource
        let resource = detector.detect(Duration::from_secs(0));

        // Verify that the resource contains the expected values
        assert_eq!(resource.get(&resource::K8S_POD_NAME).unwrap().to_string(), "test-pod");
        assert_eq!(resource.get(&resource::K8S_NAMESPACE_NAME).unwrap().to_string(), "test-namespace");
        assert_eq!(resource.get(&resource::K8S_NODE_NAME).unwrap().to_string(), "test-node");
    }

    #[test]
    fn test_k8s_resource_detector_without_env_vars() {
        // Unset environment variables to simulate a missing environment
        env::remove_var("K8S_POD_NAME");
        env::remove_var("K8S_NAMESPACE_NAME");
        env::remove_var("K8S_NODE_NAME");

        // Create an instance of K8sResourceDetector
        let detector = K8sResourceDetector;

        // Call detect function to get the Resource
        let resource = detector.detect(Duration::from_secs(0));

        // Verify that the resource uses default "unknown" values
        assert_eq!(resource.get(&resource::K8S_POD_NAME).unwrap().to_string(), "unknown_pod");
        assert_eq!(resource.get(&resource::K8S_NAMESPACE_NAME).unwrap().to_string(), "unknown_namespace");
        assert_eq!(resource.get(&resource::K8S_NODE_NAME).unwrap().to_string(), "unknown_node");
    }
}
