//! # opentelemetry-c-sdk
//!
//! The **C SDK** of the `opentelemetry-c` split: an OTLP HTTP/protobuf exporter and batch
//! span processor behind the `otel_sdk_*` C functions. Installing as global (or fetching a
//! provider handle) registers this SDK's implementation into the **API cdylib's** global
//! provider slot across the C ABI, so API-only instrumentation observes it.
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
mod error;
mod handle;
mod sdk;
mod vtable;

pub use sdk::{
    otel_sdk_build, otel_sdk_builder_add_otlp_header, otel_sdk_builder_add_resource_attribute,
    otel_sdk_builder_destroy, otel_sdk_builder_new,
    otel_sdk_builder_set_batch_export_timeout_millis,
    otel_sdk_builder_set_batch_max_export_batch_size, otel_sdk_builder_set_batch_max_queue_size,
    otel_sdk_builder_set_batch_scheduled_delay_millis, otel_sdk_builder_set_otlp_endpoint,
    otel_sdk_builder_set_otlp_timeout_millis, otel_sdk_builder_set_service_name, otel_sdk_destroy,
    otel_sdk_force_flush, otel_sdk_get_tracer_provider, otel_sdk_set_as_global, otel_sdk_shutdown,
    OtelSdk, OtelSdkBuilder,
};
