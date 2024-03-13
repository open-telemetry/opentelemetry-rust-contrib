//! Representations of entities producing telemetry.
//! ["standard attributes"]: https://github.com/open-telemetry/opentelemetry-specification/blob/v1.9.0/specification/resource/semantic_conventions/README.md
//!
//! # Resource detectors
//!
//! [`ResourceDetector`]s are used to detect resource from runtime or
//! environmental variables.
//!
//! - [`OsResourceDetector`] - detect OS from runtime.
//! - [`ProcessResourceDetector`] - detect process information.
mod os;
mod process;

pub use os::OsResourceDetector;
pub use process::ProcessResourceDetector;
