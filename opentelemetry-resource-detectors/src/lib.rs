//! Representations of entities producing telemetry.
//! ["standard attributes"]: <https://github.com/open-telemetry/opentelemetry-specification/blob/v1.9.0/specification/resource/semantic_conventions/README.md>
//!
//! # Resource detectors
//!
//! - [`OsResourceDetector`] - detect OS from runtime.
//! - [`ProcessResourceDetector`] - detect process information.
//! - [`HostResourceDetector`] - detect unique host ID.
//! - [`K8sResourceDetector`] - detect Kubernetes information.
//! - [`ContainerResourceDetector`] - detect container ID.
//! - [`ServiceInstanceIdResourceDetector`] - generate a unique service instance ID.
mod container;
mod host;
mod k8s;
mod os;
mod process;
mod service_instance;
mod uuid;

pub use container::ContainerResourceDetector;
pub use host::HostResourceDetector;
pub use k8s::K8sResourceDetector;
pub use os::OsResourceDetector;
pub use process::ProcessResourceDetector;
pub use service_instance::ServiceInstanceIdResourceDetector;
