//! # opentelemetry-c-sdk
//!
//! The **C SDK** of the `opentelemetry-c` split. It provides a trace pipeline composed of a
//! trace exporter and a span processor: an OTLP HTTP/protobuf exporter (built by
//! `otlp_exporter`) wrapped in a batch span processor (built by `batch_processor`) and
//! assembled into an `sdk` provider. Installing as global (or fetching a provider handle)
//! registers this SDK's implementation into the **API cdylib's** global provider slot across
//! the C ABI, so API-only instrumentation observes it.
//!
//! ## Linking model
//!
//! Applications link `libopentelemetry_c_sdk` **and** `libopentelemetry_c_api`. This cdylib
//! references the API's internal registration symbols (`otel_api_register_global_provider`,
//! `otel_api_provider_new`, `otel_api_set_last_error`, `otel_api_clear_last_error`), which
//! resolve against `libopentelemetry_c_api` at load time (see `build.rs`). This crate never
//! re-exports the public API/trace/common functions, so there are no duplicate symbols.

#![allow(unsafe_attr_outside_unsafe)]

// `reqwest` is a direct dependency solely to select the OTLP blocking client's TLS backend
// via the `native-tls` / `rustls-tls` cargo features; it is never called directly.
use reqwest as _;

mod api_ffi;
mod batch_processor;
mod error;
mod handle;
mod otlp_exporter;
mod sdk;
mod span_processor;
mod trace_exporter;
mod vtable;

pub use batch_processor::{
    otel_batch_span_processor_builder_build, otel_batch_span_processor_builder_destroy,
    otel_batch_span_processor_builder_new,
    otel_batch_span_processor_builder_set_export_timeout_millis,
    otel_batch_span_processor_builder_set_exporter,
    otel_batch_span_processor_builder_set_max_export_batch_size,
    otel_batch_span_processor_builder_set_max_queue_size,
    otel_batch_span_processor_builder_set_scheduled_delay_millis, OtelBatchSpanProcessorBuilder,
};
pub use otlp_exporter::{
    otel_otlp_trace_exporter_builder_add_header, otel_otlp_trace_exporter_builder_build,
    otel_otlp_trace_exporter_builder_destroy, otel_otlp_trace_exporter_builder_new,
    otel_otlp_trace_exporter_builder_set_endpoint,
    otel_otlp_trace_exporter_builder_set_timeout_millis, OtelOtlpTraceExporterBuilder,
};
pub use sdk::{
    otel_sdk_build, otel_sdk_builder_add_resource_attribute, otel_sdk_builder_add_span_processor,
    otel_sdk_builder_destroy, otel_sdk_builder_new, otel_sdk_builder_set_service_name,
    otel_sdk_destroy, otel_sdk_force_flush, otel_sdk_get_tracer_provider, otel_sdk_set_as_global,
    otel_sdk_shutdown, OtelSdk, OtelSdkBuilder,
};
pub use span_processor::{otel_span_processor_destroy, OtelSpanProcessor};
pub use trace_exporter::{otel_trace_exporter_destroy, OtelTraceExporter};
