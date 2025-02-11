//! Representations of entities producing telemetry.
//! ["standard attributes"]: <https://github.com/open-telemetry/opentelemetry-specification/blob/v1.9.0/specification/resource/semantic_conventions/README.md>
//!
//! # Resource detectors
//!
//! - [`OsResourceDetector`] - detect OS from runtime.
//! - [`ProcessResourceDetector`] - detect process information.
//! - [`HostResourceDetector`] - detect unique host ID.
//! - [`K8sResourceDetector`] - detect Kubernetes information.
mod host;
mod k8s;
mod os;
mod process;

pub use host::HostResourceDetector;
pub use k8s::K8sResourceDetector;
pub use os::OsResourceDetector;
pub use process::ProcessResourceDetector;
