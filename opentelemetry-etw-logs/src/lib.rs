//! The ETW exporter will enable applications to use OpenTelemetry API
//! to capture the telemetry events, and write them to the ETW subsystem.
//!
//! ## Resource Attribute Handling
//!
//! **Important**: By default, resource attributes are NOT exported with log records.
//! The ETW exporter only automatically exports these specific resource attributes:
//!
//! - **`service.name`** → Exported as `cloud.roleName` in PartA of Common Schema
//! - **`service.instance.id`** → Exported as `cloud.roleInstance` in PartA of Common Schema
//!
//! All other resource attributes are ignored unless explicitly specified.
//!
//! ### Opting in to Additional Resource Attributes
//!
//! To export additional resource attributes, use the `with_resource_attributes()` method:
//!
//! ```rust
//! use opentelemetry_sdk::logs::SdkLoggerProvider;
//! use opentelemetry_sdk::Resource;
//! use opentelemetry_etw_logs::Processor;
//! use opentelemetry::KeyValue;
//!
//! let etw_processor = Processor::builder("myprovider")
//!     // Only export specific resource attributes
//!     .with_resource_attributes(["custom_attribute1", "custom_attribute2"])
//!     .build()
//!     .unwrap();
//!
//! let provider = SdkLoggerProvider::builder()
//!     .with_resource(
//!         Resource::builder_empty()
//!             .with_service_name("example")
//!             .with_attribute(KeyValue::new("custom_attribute1", "value1"))
//!             .with_attribute(KeyValue::new("custom_attribute2", "value2"))
//!             .with_attribute(KeyValue::new("custom_attribute3", "value3")) // This won't be exported
//!             .build(),
//!     )
//!     .with_log_processor(etw_processor)
//!     .build();
//! ```

#![warn(missing_debug_implementations, missing_docs)]

#[cfg(feature = "serde_json")]
mod converters;
mod exporter;
mod processor;

pub use processor::Processor;
pub use processor::ProcessorBuilder;
