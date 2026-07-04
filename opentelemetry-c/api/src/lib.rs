//! # opentelemetry-c-api
//!
//! The **C API facade** of the `opentelemetry-c` split. This library exposes the public
//! trace API (tracer providers, tracers, spans), owns the single process-global provider
//! slot, and provides a **no-op default** so that API-only instrumentation is safe before
//! (or without) any SDK. It depends only on the internal [`opentelemetry_c_abi`] types
//! crate — never on `opentelemetry_sdk`, `opentelemetry-otlp`, or `reqwest`.
//!
//! ## Linking model
//!
//! - **Instrumentation libraries** link **only** `libopentelemetry_c_api`. Their trace
//!   calls are no-ops until an SDK is installed, then dispatch to it.
//! - **Applications** link `libopentelemetry_c_sdk` **and** `libopentelemetry_c_api`. The
//!   SDK registers its implementation into *this* library's global slot (across the C ABI
//!   via `otel_api_register_global_provider`), so it is visible to API-only instrumentation.
//!
//! There is exactly one global provider slot in the process — owned here — so no duplicate
//! global state exists across the two libraries.

// FFI declarations use `#[repr(C)]`/`#[no_mangle]` attributes that newer editions want
// wrapped in `unsafe(...)`; this is standard, sound FFI boilerplate.
#![allow(unsafe_attr_outside_unsafe)]

mod error;
mod global;
mod handle;
mod trace;

// Re-export the shared ABI value types so Rust consumers (and this crate's tests) can use
// them by name; these are the same `#[repr(C)]` types the C headers describe.
pub use opentelemetry_c_abi::{
    OtelAttributeType, OtelAttributeValue, OtelBool, OtelKeyValue, OtelSpanKind,
    OtelSpanStatusCode, OtelStatus, OtelStringView,
};

pub use error::{otel_api_clear_last_error, otel_api_set_last_error, otel_last_error_message};
pub use global::{
    otel_api_provider_new, otel_api_register_global_provider, otel_global_tracer_provider,
};
pub use trace::{
    otel_span_add_event, otel_span_destroy, otel_span_end, otel_span_set_attribute,
    otel_span_set_bool_attribute, otel_span_set_double_attribute, otel_span_set_int64_attribute,
    otel_span_set_status, otel_span_set_string_attribute, otel_span_update_name,
    otel_tracer_destroy, otel_tracer_provider_destroy, otel_tracer_provider_get_tracer,
    otel_tracer_start_span, OtelSpan, OtelSpanStartOptions, OtelTracer, OtelTracerProvider,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Major version component.
#[no_mangle]
pub extern "C" fn otel_version_major() -> u32 {
    handle::guard_value(0, || parse_component(0))
}

/// Minor version component.
#[no_mangle]
pub extern "C" fn otel_version_minor() -> u32 {
    handle::guard_value(0, || parse_component(1))
}

/// Patch version component.
#[no_mangle]
pub extern "C" fn otel_version_patch() -> u32 {
    handle::guard_value(0, || parse_component(2))
}

fn parse_component(index: usize) -> u32 {
    VERSION
        .split('.')
        .nth(index)
        .and_then(|s| s.split(['-', '+']).next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Full semantic version string. The returned view points at static storage.
#[no_mangle]
pub extern "C" fn otel_version_string() -> OtelStringView {
    handle::guard_value(OtelStringView::empty(), || OtelStringView {
        ptr: VERSION.as_ptr().cast::<std::os::raw::c_char>(),
        len: VERSION.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_reported() {
        let v = otel_version_string();
        assert!(!v.ptr.is_null() && v.len > 0);
        assert_eq!(otel_version_major(), 0);
        let _ = otel_version_minor();
        let _ = otel_version_patch();
    }
}
