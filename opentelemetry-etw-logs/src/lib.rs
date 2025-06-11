//! The ETW exporter will enable applications to use OpenTelemetry API
//! to capture the telemetry events, and write them to the ETW subsystem.

#![warn(missing_debug_implementations, missing_docs)]

mod logs;

pub use logs::*;
