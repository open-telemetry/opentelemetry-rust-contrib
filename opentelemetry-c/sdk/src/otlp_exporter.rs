//! The OTLP trace exporter builder (`otel_otlp_trace_exporter_builder_t`).
//!
//! Configures and builds an OTLP **HTTP/protobuf** trace exporter, producing a generic
//! [`OtelTraceExporter`] handle. HTTPS is available via a selectable TLS backend chosen at
//! compile time with the crate's `native-tls` (default) or `rustls-tls` cargo features; the
//! exporter owns its own blocking HTTP client, so no user-managed async runtime is required.

use std::collections::HashMap;
use std::time::Duration;

use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig, WithHttpConfig};

use opentelemetry_c_abi::{OtelStatus, OtelStringView};

use crate::error::{clear_last_error, fail, fail_abi, fail_owned};
use crate::handle::{
    checked_mut, checked_ref, destroy, guard_ptr, guard_status, guard_unit, into_raw, HasMagic,
};
use crate::trace_exporter::OtelTraceExporter;

const OTLP_EXPORTER_BUILDER_MAGIC: u64 = 0x4F54_4C43_4F54_4C42; // "OTLCOTLB"

#[derive(Default)]
struct OtlpExporterConfig {
    endpoint: Option<String>,
    headers: Vec<(String, String)>,
    timeout: Option<Duration>,
}

/// Opaque OTLP trace exporter builder. Not thread-safe; confine to one thread.
pub struct OtelOtlpTraceExporterBuilder {
    magic: u64,
    config: OtlpExporterConfig,
}

impl HasMagic for OtelOtlpTraceExporterBuilder {
    const MAGIC: u64 = OTLP_EXPORTER_BUILDER_MAGIC;
    fn magic(&self) -> u64 {
        self.magic
    }
    fn set_magic(&mut self, value: u64) {
        self.magic = value;
    }
}

/// Create a new OTLP trace exporter builder. Release with
/// `otel_otlp_trace_exporter_builder_destroy()`.
#[no_mangle]
pub extern "C" fn otel_otlp_trace_exporter_builder_new() -> *mut OtelOtlpTraceExporterBuilder {
    guard_ptr(|| {
        clear_last_error();
        into_raw(OtelOtlpTraceExporterBuilder {
            magic: OTLP_EXPORTER_BUILDER_MAGIC,
            config: OtlpExporterConfig::default(),
        })
    })
}

/// Destroy an OTLP trace exporter builder (no-op on NULL).
///
/// # Safety
/// `builder` must be NULL or a live builder not destroyed concurrently.
#[no_mangle]
pub unsafe extern "C" fn otel_otlp_trace_exporter_builder_destroy(
    builder: *mut OtelOtlpTraceExporterBuilder,
) {
    guard_unit(|| unsafe { destroy(builder) });
}

/// # Safety
/// `builder` must satisfy the handle contract (single-threaded).
unsafe fn with_config<F>(builder: *mut OtelOtlpTraceExporterBuilder, f: F) -> OtelStatus
where
    F: FnOnce(&mut OtlpExporterConfig) -> OtelStatus,
{
    guard_status(|| {
        clear_last_error();
        match unsafe { checked_mut(builder) } {
            Some(b) => f(&mut b.config),
            None => OtelStatus::InvalidArgument,
        }
    })
}

/// Set the full OTLP traces endpoint URL, used as-is (e.g.
/// `http://localhost:4318/v1/traces`).
///
/// # Safety
/// `builder` and `endpoint` must satisfy their contracts.
#[no_mangle]
pub unsafe extern "C" fn otel_otlp_trace_exporter_builder_set_endpoint(
    builder: *mut OtelOtlpTraceExporterBuilder,
    endpoint: OtelStringView,
) -> OtelStatus {
    unsafe {
        with_config(builder, |config| match endpoint.to_string_strict() {
            Ok(endpoint) => {
                config.endpoint = Some(endpoint);
                OtelStatus::Ok
            }
            Err(e) => fail_abi(e),
        })
    }
}

/// Add an HTTP header sent with every OTLP export request.
///
/// Duplicate keys are rejected case-insensitively: if `key` (after strict UTF-8 conversion)
/// matches an already-added key under ASCII case-insensitive comparison — so `Authorization`
/// and `authorization` collide — the call fails with `OTEL_STATUS_INVALID_ARGUMENT` and leaves
/// the configuration unchanged, rather than silently overwriting the earlier value.
///
/// # Safety
/// `builder`, `key`, `value` must satisfy their contracts.
#[no_mangle]
pub unsafe extern "C" fn otel_otlp_trace_exporter_builder_add_header(
    builder: *mut OtelOtlpTraceExporterBuilder,
    key: OtelStringView,
    value: OtelStringView,
) -> OtelStatus {
    unsafe {
        with_config(builder, |config| {
            let key = match key.to_string_strict() {
                Ok(k) if !k.is_empty() => k,
                Ok(_) => {
                    return fail(
                        OtelStatus::InvalidArgument,
                        "OTLP header key must not be empty",
                    )
                }
                Err(e) => return fail_abi(e),
            };
            if config
                .headers
                .iter()
                .any(|(existing, _)| existing.eq_ignore_ascii_case(&key))
            {
                return fail_owned(
                    OtelStatus::InvalidArgument,
                    format!("OTLP header key already exists: {key}"),
                );
            }
            let value = match value.to_string_strict() {
                Ok(v) => v,
                Err(e) => return fail_abi(e),
            };
            config.headers.push((key, value));
            OtelStatus::Ok
        })
    }
}

/// Set the OTLP export request timeout in milliseconds (`0` == exporter default).
///
/// # Safety
/// `builder` must satisfy the handle contract.
#[no_mangle]
pub unsafe extern "C" fn otel_otlp_trace_exporter_builder_set_timeout_millis(
    builder: *mut OtelOtlpTraceExporterBuilder,
    timeout_millis: u64,
) -> OtelStatus {
    unsafe {
        with_config(builder, |config| {
            config.timeout = (timeout_millis != 0).then(|| Duration::from_millis(timeout_millis));
            OtelStatus::Ok
        })
    }
}

fn build_otlp_exporter(config: &OtlpExporterConfig) -> Result<SpanExporter, OtelStatus> {
    let mut builder = SpanExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary);
    if let Some(endpoint) = &config.endpoint {
        builder = builder.with_endpoint(endpoint.clone());
    }
    if let Some(timeout) = config.timeout {
        builder = builder.with_timeout(timeout);
    }
    if !config.headers.is_empty() {
        let headers: HashMap<String, String> = config.headers.iter().cloned().collect();
        builder = builder.with_headers(headers);
    }
    builder.build().map_err(|err| {
        fail_owned(
            OtelStatus::InvalidConfig,
            format!("failed to build OTLP exporter: {err}"),
        )
    })
}

/// Build a trace exporter from the accumulated configuration. On `OTEL_STATUS_OK`, `*out`
/// receives a new [`OtelTraceExporter`] handle owned by the caller (release with
/// `otel_trace_exporter_destroy`, or transfer it into a span processor builder). The builder
/// remains owned by the caller.
///
/// # Safety
/// `builder` must satisfy the handle contract; `out` a valid writable
/// `otel_trace_exporter_t*`.
#[no_mangle]
pub unsafe extern "C" fn otel_otlp_trace_exporter_builder_build(
    builder: *const OtelOtlpTraceExporterBuilder,
    out: *mut *mut OtelTraceExporter,
) -> OtelStatus {
    guard_status(|| {
        clear_last_error();
        if out.is_null() {
            return fail(OtelStatus::InvalidArgument, "out pointer must not be NULL");
        }
        unsafe { *out = std::ptr::null_mut() };
        let builder = match unsafe { checked_ref(builder) } {
            Some(b) => b,
            None => return OtelStatus::InvalidArgument,
        };
        let exporter = match build_otlp_exporter(&builder.config) {
            Ok(e) => e,
            Err(status) => return status,
        };
        unsafe { *out = into_raw(OtelTraceExporter::new(exporter)) };
        OtelStatus::Ok
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace_exporter::otel_trace_exporter_destroy;

    fn sv(s: &str) -> OtelStringView {
        OtelStringView {
            ptr: s.as_ptr() as *const std::os::raw::c_char,
            len: s.len(),
        }
    }

    #[test]
    fn setters_and_build_succeed() {
        unsafe {
            let eb = otel_otlp_trace_exporter_builder_new();
            assert_eq!(
                otel_otlp_trace_exporter_builder_set_endpoint(
                    eb,
                    sv("http://127.0.0.1:4318/v1/traces")
                ),
                OtelStatus::Ok
            );
            assert_eq!(
                otel_otlp_trace_exporter_builder_add_header(
                    eb,
                    sv("authorization"),
                    sv("Bearer x")
                ),
                OtelStatus::Ok
            );
            // An empty header key is rejected.
            assert_eq!(
                otel_otlp_trace_exporter_builder_add_header(eb, sv(""), sv("v")),
                OtelStatus::InvalidArgument
            );
            assert_eq!(
                otel_otlp_trace_exporter_builder_set_timeout_millis(eb, 3000),
                OtelStatus::Ok
            );
            let mut exporter: *mut OtelTraceExporter = std::ptr::null_mut();
            assert_eq!(
                otel_otlp_trace_exporter_builder_build(eb, &mut exporter),
                OtelStatus::Ok
            );
            assert!(!exporter.is_null());
            otel_otlp_trace_exporter_builder_destroy(eb);
            otel_trace_exporter_destroy(exporter);
        }
    }

    #[test]
    fn duplicate_header_key_is_rejected() {
        unsafe {
            let eb = otel_otlp_trace_exporter_builder_new();
            // First occurrence of a key is accepted.
            assert_eq!(
                otel_otlp_trace_exporter_builder_add_header(eb, sv("authorization"), sv("first")),
                OtelStatus::Ok
            );
            // A second add of the SAME key differing only in ASCII case is rejected
            // case-insensitively (no silent overwrite) ...
            assert_eq!(
                otel_otlp_trace_exporter_builder_add_header(eb, sv("Authorization"), sv("second")),
                OtelStatus::InvalidArgument
            );
            // ... with a clear last-error message ...
            assert!(crate::api_ffi::test_probe::last_error().contains("already exists"));
            // ... while a different key is still accepted.
            assert_eq!(
                otel_otlp_trace_exporter_builder_add_header(eb, sv("x-custom"), sv("v")),
                OtelStatus::Ok
            );
            // The builder still builds (the retained first value was never overwritten).
            let mut exporter: *mut OtelTraceExporter = std::ptr::null_mut();
            assert_eq!(
                otel_otlp_trace_exporter_builder_build(eb, &mut exporter),
                OtelStatus::Ok
            );
            assert!(!exporter.is_null());
            otel_otlp_trace_exporter_builder_destroy(eb);
            otel_trace_exporter_destroy(exporter);
        }
    }

    #[test]
    fn build_with_null_out_is_invalid() {
        unsafe {
            let eb = otel_otlp_trace_exporter_builder_new();
            assert_eq!(
                otel_otlp_trace_exporter_builder_build(eb, std::ptr::null_mut()),
                OtelStatus::InvalidArgument
            );
            otel_otlp_trace_exporter_builder_destroy(eb);
        }
    }
}
