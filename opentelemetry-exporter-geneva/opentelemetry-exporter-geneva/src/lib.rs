//! The OpenTelemetry Geneva exporter will enable applications to use OpenTelemetry API
//! to capture the telemetry events, and write to Microsoft internal backend.

#![warn(missing_debug_implementations, missing_docs)]

mod logs;

pub use logs::*;
