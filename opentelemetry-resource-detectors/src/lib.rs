//! Representations of entities producing telemetry.
//! ["standard attributes"]: <https://github.com/open-telemetry/opentelemetry-specification/blob/v1.9.0/specification/resource/semantic_conventions/README.md>
//!
//! # Resource detectors
//!
//! - [`OsResourceDetector`] - detect OS from runtime.
//! - [`ProcessResourceDetector`] - detect process information.
//! - [`HostResourceDetector`] - detect unique host ID.
//! - [`LambdaResourceDetector`] - detect resource metadata from an AWS Lambda environment.
mod aws;
mod host;
mod os;
mod process;

#[cfg(feature = "aws-lambda")]
pub use aws::LambdaResourceDetector;
pub use host::HostResourceDetector;
pub use os::OsResourceDetector;
pub use process::ProcessResourceDetector;
