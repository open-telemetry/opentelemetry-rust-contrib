//! The ETW exporter will enable applications to use OpenTelemetry API
//! to capture the telemetry events, and write them to the ETW subsystem.

#![warn(missing_debug_implementations, missing_docs)]

#[cfg(feature = "serde_json")]
mod converters;
mod exporter;
mod processor;

pub use processor::Processor;
pub use processor::ProcessorBuilder;
