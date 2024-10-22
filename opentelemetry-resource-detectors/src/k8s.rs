use opentelemetry::sdk::resource::{Resource, ResourceDetector};
use opentelemetry_semantic_conventions::resource;
use std::env;
use std::time::Duration;

/// A resource detector for Kubernetes environment variables.
pub struct K8sResourceDetector;

impl ResourceDetector for K8sResourceDetector {
    /// Detect Kubernetes-related environment variables and return a Resource.
    fn detect(&self, _timeout: Duration) -> Resource {
        // Attempt to read Kubernetes-specific environment variables.
        let pod_name = env::var("K8S_POD_NAME").unwrap_or_else(|_| "unknown_pod".to_string());
        let namespace_name = env::var("K8S_NAMESPACE_NAME").unwrap_or_else(|_| "unknown_namespace".to_string());
        let node_name = env::var("K8S_NODE_NAME").unwrap_or_else(|_| "unknown_node".to_string());

        // Create a Resource with Kubernetes attributes.
        Resource::new(vec![
            resource::K8S_POD_NAME.string(pod_name),
            resource::K8S_NAMESPACE_NAME.string(namespace_name),
            resource::K8S_NODE_NAME.string(node_name),
        ])
    }
}
