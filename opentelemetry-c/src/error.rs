//! Status codes and thread-local error reporting for the C API.
//!
//! Every fallible entry point returns an [`OtelStatus`]. When a non-`Ok` status is
//! returned, a human-readable diagnostic message is stored in a thread-local slot
//! that can be retrieved with [`otel_last_error_message`]. The message is only valid
//! until the next OpenTelemetry C call on the same thread.

use std::cell::RefCell;
use std::ffi::CString;

use opentelemetry_sdk::error::OTelSdkError;

use crate::common::OtelStringView;

/// Status code returned by fallible C API functions.
///
/// `OTEL_STATUS_OK` (0) indicates success. All other values indicate failure and a
/// diagnostic string is available from `otel_last_error_message()`.
///
/// New variants may be appended in future minor releases; C callers should treat any
/// unknown non-zero value as a generic failure.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtelStatus {
    /// Operation completed successfully.
    Ok = 0,
    /// A required pointer argument was NULL, or a handle failed validation.
    InvalidArgument = 1,
    /// A string argument was not valid UTF-8 where UTF-8 is required.
    InvalidUtf8 = 2,
    /// Configuration supplied to the SDK builder was invalid (e.g. bad endpoint).
    InvalidConfig = 3,
    /// The SDK (or provider) has already been shut down.
    AlreadyShutdown = 4,
    /// The operation did not complete within the supplied timeout.
    Timeout = 5,
    /// A span export failed at runtime. This never crashes the process.
    ExportFailed = 6,
    /// An unexpected internal error occurred (including a caught Rust panic).
    InternalError = 7,
}

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

/// Record a diagnostic message for the current thread's last error slot.
pub(crate) fn set_last_error(message: impl Into<Vec<u8>>) {
    // Sanitize interior NULs so the value is always a valid C string.
    let bytes = message.into();
    let cstring = CString::new(bytes).unwrap_or_else(|e| {
        let mut v = e.into_vec();
        v.retain(|&b| b != 0);
        // SAFETY: all NUL bytes were removed above.
        unsafe { CString::from_vec_unchecked(v) }
    });
    LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(cstring));
}

/// Clear the current thread's last error slot.
pub(crate) fn clear_last_error() {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = None);
}

/// Convenience helper: record `message` and return `status`.
pub(crate) fn fail(status: OtelStatus, message: impl Into<Vec<u8>>) -> OtelStatus {
    set_last_error(message);
    status
}

/// Map an [`OTelSdkError`] onto a C status code, recording the detail message.
pub(crate) fn status_from_sdk_error(err: &OTelSdkError) -> OtelStatus {
    match err {
        OTelSdkError::AlreadyShutdown => {
            set_last_error("operation failed: provider already shut down");
            OtelStatus::AlreadyShutdown
        }
        OTelSdkError::Timeout(d) => {
            set_last_error(format!("operation timed out after {d:?}"));
            OtelStatus::Timeout
        }
        OTelSdkError::InternalFailure(msg) => {
            set_last_error(format!("internal failure: {msg}"));
            OtelStatus::ExportFailed
        }
    }
}

/// Retrieve the last error message recorded on the calling thread.
///
/// The returned view points at thread-local storage that remains valid until the next
/// OpenTelemetry C call on the same thread. If no error has been recorded, the returned
/// view has a NULL `ptr` and zero `len`.
///
/// # Safety
/// This function is safe to call at any time, including with no prior error.
#[no_mangle]
pub extern "C" fn otel_last_error_message() -> OtelStringView {
    crate::handle::guard_value(OtelStringView::empty(), || {
        LAST_ERROR.with(|slot| match &*slot.borrow() {
            Some(cstring) => {
                let bytes = cstring.as_bytes();
                // Pointer remains valid until this thread's slot is overwritten.
                OtelStringView {
                    ptr: bytes.as_ptr() as *const std::os::raw::c_char,
                    len: bytes.len(),
                }
            }
            None => OtelStringView::empty(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn last_error_roundtrip() {
        clear_last_error();
        let empty = otel_last_error_message();
        assert!(empty.ptr.is_null());
        assert_eq!(empty.len, 0);

        set_last_error("boom");
        let view = otel_last_error_message();
        assert!(!view.ptr.is_null());
        // SAFETY: the view points at the thread-local CString which is still alive.
        let bytes = unsafe { std::slice::from_raw_parts(view.ptr as *const u8, view.len) };
        assert_eq!(bytes, b"boom");

        clear_last_error();
        assert!(otel_last_error_message().ptr.is_null());
    }

    #[test]
    fn interior_nul_is_sanitized() {
        clear_last_error();
        set_last_error("a\0b");
        let view = otel_last_error_message();
        // SAFETY: the view points at the live thread-local CString.
        let bytes = unsafe { std::slice::from_raw_parts(view.ptr as *const u8, view.len) };
        assert!(!bytes.contains(&0));
        clear_last_error();
    }

    #[test]
    fn sdk_error_mapping() {
        assert_eq!(
            status_from_sdk_error(&OTelSdkError::AlreadyShutdown),
            OtelStatus::AlreadyShutdown
        );
        assert_eq!(
            status_from_sdk_error(&OTelSdkError::Timeout(Duration::from_secs(1))),
            OtelStatus::Timeout
        );
        assert_eq!(
            status_from_sdk_error(&OTelSdkError::InternalFailure("x".to_owned())),
            OtelStatus::ExportFailed
        );
    }

    #[test]
    fn fail_records_message_and_returns_status() {
        clear_last_error();
        let status = fail(OtelStatus::InvalidConfig, "nope");
        assert_eq!(status, OtelStatus::InvalidConfig);
        assert!(!otel_last_error_message().ptr.is_null());
        clear_last_error();
    }
}
