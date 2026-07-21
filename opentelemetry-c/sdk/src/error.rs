//! SDK-side error helpers. Diagnostics are recorded in the **API-owned** thread-local slot
//! (via the internal ABI in [`crate::api_ffi`]) so a subsequent `otel_last_error_message()`
//! returns them, exactly as in the single-crate build.

use opentelemetry_c_abi::{AbiError, OtelStatus};
use opentelemetry_sdk::error::OTelSdkError;

use crate::api_ffi;

/// Record `message` in the API error slot and return `status`.
pub(crate) fn fail(status: OtelStatus, message: &str) -> OtelStatus {
    api_ffi::set_last_error(message);
    status
}

/// Record an owned `message` and return `status`.
pub(crate) fn fail_owned(status: OtelStatus, message: String) -> OtelStatus {
    api_ffi::set_last_error(&message);
    status
}

/// Clear the API error slot (called at the start of fallible entry points).
pub(crate) fn clear_last_error() {
    api_ffi::clear_last_error();
}

/// Map an [`AbiError`] onto a status, recording its message.
pub(crate) fn fail_abi(err: AbiError) -> OtelStatus {
    fail(err.status, err.message)
}

/// Map an [`OTelSdkError`] onto a status, recording the detail message.
pub(crate) fn status_from_sdk_error(err: &OTelSdkError) -> OtelStatus {
    match err {
        OTelSdkError::AlreadyShutdown => fail(
            OtelStatus::AlreadyShutdown,
            "operation failed: provider already shut down",
        ),
        OTelSdkError::Timeout(d) => fail_owned(
            OtelStatus::Timeout,
            format!("operation timed out after {d:?}"),
        ),
        OTelSdkError::InternalFailure(msg) => {
            fail_owned(OtelStatus::ExportFailed, format!("internal failure: {msg}"))
        }
    }
}
