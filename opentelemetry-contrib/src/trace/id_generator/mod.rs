//! # Opentelemetry id generator contrib
//!
//!
#[cfg(feature = "xray_id_generator")]
mod aws;
#[cfg(feature = "xray_id_generator")]
pub use aws::XrayIdGenerator;
