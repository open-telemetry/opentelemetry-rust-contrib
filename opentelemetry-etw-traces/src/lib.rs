//! # OpenTelemetry ETW Traces Exporter
//!
//! Exports OpenTelemetry span data to [Event Tracing for Windows (ETW)][etw]
//! using the [TraceLogging Dynamic][tld] format with
//! [Microsoft Common Schema v4.0][cs] encoding.
//!
//! This crate provides a [`SpanProcessor`] implementation that writes
//! completed spans directly to ETW, suitable for integration with the
//! OpenTelemetry SDK's [`SdkTracerProvider`].
//!
//! ## Usage
//!
//! ```no_run
//! use opentelemetry_etw_traces::Processor;
//! use opentelemetry_sdk::trace::SdkTracerProvider;
//!
//! let processor = Processor::builder("MyAppTracing")
//!     .with_event_name("MyAppEventName") // If not provided, defaults to "Span"
//!     .with_resource_attributes(vec!["custom_attribute1", "custom_attribute2"]) // Only specified resource attributes will be promoted as Part C fields, other will be ignored.
//!     .build()
//!     .expect("Failed to create ETW processor");
//!
//! let provider = SdkTracerProvider::builder()
//!     .with_span_processor(processor)
//!     .build();
//! ```
//!
//! ## Common Schema Mapping
//!
//! Each span is written as an ETW event following
//! the TraceLoggingDynamic format with the following structure:
//!
//! - **`__csver__`**: Common Schema version (`u16`, value `1024` / `0x0400`)
//! - **Part A** (envelope): `time`, `ext_dt { traceId, spanId }`,
//!   `cloud.role`, `cloud.roleInstance`
//! - **Part B** (payload): `_typeName="Span"`, `name`, `kind`, `startTime`,
//!   `parentId`, `links`, `statusMessage`, `success`
//! - **Part C** (extensions): promoted resource attributes, span's attributes, `events`
//!
//! ## Platform Support
//!
//! This crate is Windows-only, as ETW is a Windows-specific tracing facility.
//!
//! [etw]: https://learn.microsoft.com/en-us/windows/win32/etw/about-event-tracing
//! [tld]: https://crates.io/crates/tracelogging_dynamic
//! [cs]: https://learn.microsoft.com/en-us/opentelemetry/common-schema
//! [`SpanProcessor`]: opentelemetry_sdk::trace::SpanProcessor
//! [`SdkTracerProvider`]: opentelemetry_sdk::trace::SdkTracerProvider

#![warn(missing_debug_implementations, missing_docs)]

mod exporter;
mod processor;

pub use processor::Processor;
pub use processor::ProcessorBuilder;
