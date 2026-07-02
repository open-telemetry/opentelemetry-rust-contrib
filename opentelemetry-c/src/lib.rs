//! # opentelemetry-c
//!
//! A Rust-backed C API and SDK for [OpenTelemetry](https://opentelemetry.io), exposing
//! an experimental C ABI for distributed tracing. The Rust OpenTelemetry SDK does all of
//! the real work behind an opaque-handle C facade; C callers never see Rust types,
//! ownership rules, or async runtimes.
//!
//! ## What this crate provides (traces)
//!
//! - An SDK builder to configure a resource, an OTLP **HTTP/protobuf** exporter, and a
//!   batch span processor.
//! - A tracer-provider / tracer / span object model exposed as opaque C handles.
//! - Global provider installation, span creation with attributes/events/status, force
//!   flush, and shutdown.
//!
//! The exporter uses the blocking `reqwest` HTTP client, so the SDK owns all of its own
//! threading and **no user-managed async runtime is required**. HTTPS is supported via
//! a selectable TLS backend: the `native-tls` feature (default, platform TLS) or the
//! `rustls-tls` feature (pure-Rust TLS).
//!
//! ## ABI and safety model
//!
//! - Every entry point is `extern "C"`, `#[no_mangle]`, and wrapped in a panic firewall
//!   (`catch_unwind`); a Rust panic can never cross the C boundary.
//! - Handles are opaque pointers tagged with a per-type magic number, checked as a
//!   best-effort diagnostic; callers must pass NULL or a live handle of the exact expected
//!   type (wrong-type, freed, or foreign pointers are undefined behavior to pass).
//! - Fallible functions return an [`error::OtelStatus`]; functions that create handles
//!   return a nullable pointer. A diagnostic string is available from
//!   `otel_last_error_message()`.
//! - Every `*_new` / `*_get_*` / `*_start_*` function has a matching `*_destroy`.
//!   Passing `NULL` to a destructor is always safe.
//!
//! ## Threading & lifecycle contract
//!
//! - **SDK, tracer-provider, and tracer** handles may be used concurrently from multiple
//!   threads. Every SDK operation other than destruction takes a shared `&` view
//!   internally (the provider is `Arc`-backed and lifecycle state is atomic), so
//!   concurrent C callers never create `&mut` aliasing.
//! - A **span** handle is mutated through `&mut` and must not be used concurrently from
//!   multiple threads; use one span per thread or synchronize externally. Distinct spans
//!   are independent.
//! - A **builder** handle is mutated through `&mut` and is not thread-safe; confine it to
//!   one thread.
//! - **`*_destroy` must not race** with any other call on the same handle (handles are
//!   not reference-counted).
//! - `otel_sdk_shutdown` runs the underlying provider shutdown at most once (atomic
//!   guard); a timed `otel_sdk_force_flush` bounds itself to a single in-flight helper
//!   thread. No callback ever crosses the C ABI.
//!
//! The C headers under `include/opentelemetry_c/` are the source of truth for C users;
//! this crate keeps them in sync by hand (see `README.md` for the optional `cbindgen`
//! workflow).

// FFI declarations use `#[repr(C)]`/`#[no_mangle]` attributes that newer editions want
// wrapped in `unsafe(...)`; this is standard, sound FFI boilerplate.
#![allow(unsafe_attr_outside_unsafe)]

// `reqwest` is a direct dependency solely so this crate can select the OTLP blocking
// HTTP client's TLS backend through the `native-tls` / `rustls-tls` cargo features.
// The exporter is driven via `opentelemetry-otlp`; reqwest is never called directly.
use reqwest as _;

mod common;
mod error;
mod handle;
mod sdk;
mod trace;

pub use common::{OtelAttributeType, OtelAttributeValue, OtelBool, OtelKeyValue, OtelStringView};
pub use error::{otel_last_error_message, OtelStatus};
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
pub use trace::{
    otel_global_tracer_provider, otel_span_add_event, otel_span_destroy, otel_span_end,
    otel_span_set_attribute, otel_span_set_bool_attribute, otel_span_set_double_attribute,
    otel_span_set_int64_attribute, otel_span_set_status, otel_span_set_string_attribute,
    otel_span_update_name, otel_tracer_destroy, otel_tracer_provider_destroy,
    otel_tracer_provider_get_tracer, otel_tracer_start_span, OtelSpan, OtelSpanKind,
    OtelSpanStartOptions, OtelSpanStatusCode, OtelTracer, OtelTracerProvider,
};

/// Full crate version string (semver), e.g. `"0.1.0"`, as a static, NUL-terminated
/// string exposed through a length-delimited view.
const VERSION_CSTR: &str = concat!(env!("CARGO_PKG_VERSION"), "\0");

/// Major component of the library version.
#[no_mangle]
pub extern "C" fn otel_version_major() -> u32 {
    handle::guard_value(0, || env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap_or(0))
}

/// Minor component of the library version.
#[no_mangle]
pub extern "C" fn otel_version_minor() -> u32 {
    handle::guard_value(0, || env!("CARGO_PKG_VERSION_MINOR").parse().unwrap_or(0))
}

/// Patch component of the library version.
#[no_mangle]
pub extern "C" fn otel_version_patch() -> u32 {
    handle::guard_value(0, || env!("CARGO_PKG_VERSION_PATCH").parse().unwrap_or(0))
}

/// Return the full semantic version as a length-delimited, NUL-terminated static
/// string. The pointer is valid for the entire lifetime of the process.
#[no_mangle]
pub extern "C" fn otel_version_string() -> OtelStringView {
    OtelStringView {
        ptr: VERSION_CSTR.as_ptr() as *const std::os::raw::c_char,
        // Exclude the trailing NUL from the reported length.
        len: VERSION_CSTR.len() - 1,
    }
}
