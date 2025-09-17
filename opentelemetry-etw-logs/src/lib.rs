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
//!             .with_attribute(KeyValue::new("custom_attribute3", "value3"))  // This won't be exported
//!             .build(),
//!     )
//!     .with_log_processor(etw_processor)
//!     .build();
//! ```
//!
//! ### Performance Considerations for ETW
//!
//! **Warning**: Each specified resource attribute will be serialized and sent
//! with EVERY log record. This is different from OTLP exporters where resource
//! attributes are serialized once per batch. Consider the performance impact
//! when selecting which attributes to export.
//!
//! **Recommendation**: Be selective about which resource attributes to export.
//! Since ETW writes to a local kernel buffer and requires a local
//! listener/agent, the agent can often deduce many resource attributes without
//! requiring them to be sent with each log:
//!
//! - **Infrastructure attributes** (datacenter, region, availability zone) can
//!   be determined by the local agent.
//! - **Host attributes** (hostname, IP address, OS version) are available locally.
//! - **Deployment attributes** (environment, cluster) may be known to the agent.
//!
//! Focus on attributes that are truly specific to your application instance
//! and cannot be easily determined by the local agent.

#![warn(missing_debug_implementations, missing_docs)]

#[cfg(feature = "serde_json")]
mod converters;
mod exporter;
mod processor;

pub use processor::Processor;
pub use processor::ProcessorBuilder;
