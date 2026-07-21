//! Status codes and thread-local error reporting for the C API.
//!
//! The **API cdylib owns** the single thread-local error slot and exports
//! `otel_last_error_message()`. So that SDK-side failures (from the separate SDK cdylib)
//! surface through the same query, the API also exports `otel_api_set_last_error()` /
//! `otel_api_clear_last_error()`, which the SDK calls across the C ABI. This keeps one
//! diagnostic slot per thread, owned by the API, with no duplicate symbols.

use std::cell::RefCell;
use std::ffi::CString;
use std::os::raw::c_char;

pub use opentelemetry_c_abi::OtelStatus;
use opentelemetry_c_abi::OtelStringView;

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

/// Record a diagnostic message for the current thread's last-error slot.
pub(crate) fn set_last_error(message: impl Into<Vec<u8>>) {
    let bytes = message.into();
    let cstring = CString::new(bytes).unwrap_or_else(|e| {
        let mut v = e.into_vec();
        v.retain(|&b| b != 0);
        // SAFETY: all NUL bytes were removed above.
        unsafe { CString::from_vec_unchecked(v) }
    });
    LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(cstring));
}

/// Clear the current thread's last-error slot.
pub(crate) fn clear_last_error() {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = None);
}

/// Whether the calling thread currently has a recorded last-error message.
pub(crate) fn has_last_error() -> bool {
    LAST_ERROR.with(|slot| slot.borrow().is_some())
}

/// Convenience: record `message` and return `status`.
pub(crate) fn fail(status: OtelStatus, message: impl Into<Vec<u8>>) -> OtelStatus {
    set_last_error(message);
    status
}

/// Retrieve the last error message recorded on the calling thread.
///
/// The returned view points at thread-local storage valid until the next OpenTelemetry C
/// call on the same thread. With no recorded error the view is NULL / zero-length.
///
/// # Safety
/// Safe to call at any time.
#[no_mangle]
pub extern "C" fn otel_last_error_message() -> OtelStringView {
    crate::handle::guard_value(OtelStringView::empty(), || {
        LAST_ERROR.with(|slot| match &*slot.borrow() {
            Some(cstring) => {
                let bytes = cstring.as_bytes();
                OtelStringView {
                    ptr: bytes.as_ptr().cast::<c_char>(),
                    len: bytes.len(),
                }
            }
            None => OtelStringView::empty(),
        })
    })
}

/// **Internal ABI (called by the SDK cdylib).** Record a diagnostic in the API-owned
/// thread-local slot so a subsequent `otel_last_error_message()` returns it. A NULL/zero
/// view clears the slot. This is not part of the public C API.
///
/// # Safety
/// `ptr` must be NULL or point to `len` readable bytes (`len <= isize::MAX`).
#[no_mangle]
pub unsafe extern "C" fn otel_api_set_last_error(ptr: *const c_char, len: usize) {
    crate::handle::guard_unit(|| {
        if ptr.is_null() || len == 0 || len > isize::MAX as usize {
            clear_last_error();
            return;
        }
        // SAFETY: validated non-NULL with len within isize::MAX per the caller contract.
        let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) };
        set_last_error(bytes.to_vec());
    });
}

/// **Internal ABI (called by the SDK cdylib).** Clear the API-owned thread-local slot.
#[no_mangle]
pub extern "C" fn otel_api_clear_last_error() {
    crate::handle::guard_unit(clear_last_error);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_error_roundtrip() {
        clear_last_error();
        assert!(otel_last_error_message().ptr.is_null());
        set_last_error("boom");
        let view = otel_last_error_message();
        // SAFETY: view points at the live thread-local CString.
        let bytes = unsafe { std::slice::from_raw_parts(view.ptr.cast::<u8>(), view.len) };
        assert_eq!(bytes, b"boom");
        clear_last_error();
        assert!(otel_last_error_message().ptr.is_null());
    }

    #[test]
    fn api_set_last_error_from_bytes() {
        clear_last_error();
        let msg = b"sdk-said-this";
        unsafe { otel_api_set_last_error(msg.as_ptr().cast::<c_char>(), msg.len()) };
        let view = otel_last_error_message();
        let bytes = unsafe { std::slice::from_raw_parts(view.ptr.cast::<u8>(), view.len) };
        assert_eq!(bytes, msg);
        // NULL clears.
        unsafe { otel_api_set_last_error(std::ptr::null(), 0) };
        assert!(otel_last_error_message().ptr.is_null());
    }
}
