//! Handle plumbing for the SDK crate's own handles (`otel_sdk_builder_t`, `otel_sdk_t`).
//!
//! Mirrors the API crate's handle plumbing, but diagnostics are recorded in the API-owned
//! error slot via [`crate::api_ffi`] so `otel_last_error_message()` (exported by the API)
//! returns them.

use std::panic::{catch_unwind, AssertUnwindSafe};

use opentelemetry_c_abi::OtelStatus;

use crate::api_ffi;

pub(crate) trait HasMagic {
    const MAGIC: u64;
    fn magic(&self) -> u64;
    fn set_magic(&mut self, value: u64);
}

/// # Safety
/// `ptr` must be NULL or a live handle of the exact type `T`, not destroyed concurrently.
pub(crate) unsafe fn checked_ref<'a, T: HasMagic>(ptr: *const T) -> Option<&'a T> {
    if ptr.is_null() {
        api_ffi::set_last_error("null handle passed to OpenTelemetry C API");
        return None;
    }
    let handle = unsafe { &*ptr };
    if handle.magic() != T::MAGIC {
        api_ffi::set_last_error("handle failed validation: not a live handle of the expected type");
        return None;
    }
    Some(handle)
}

/// # Safety
/// `ptr` must be NULL or a live, uniquely-borrowed handle of the exact type `T`.
pub(crate) unsafe fn checked_mut<'a, T: HasMagic>(ptr: *mut T) -> Option<&'a mut T> {
    if ptr.is_null() {
        api_ffi::set_last_error("null handle passed to OpenTelemetry C API");
        return None;
    }
    let handle = unsafe { &mut *ptr };
    if handle.magic() != T::MAGIC {
        api_ffi::set_last_error("handle failed validation: not a live handle of the expected type");
        return None;
    }
    Some(handle)
}

pub(crate) fn into_raw<T>(value: T) -> *mut T {
    Box::into_raw(Box::new(value))
}

/// # Safety
/// `ptr` must be NULL or a pointer from [`into_raw`] for the same `T`, not double-freed.
pub(crate) unsafe fn destroy<T: HasMagic>(ptr: *mut T) {
    if ptr.is_null() {
        return;
    }
    let handle = unsafe { &mut *ptr };
    if handle.magic() != T::MAGIC {
        return;
    }
    handle.set_magic(0);
    drop(unsafe { Box::from_raw(ptr) });
}

pub(crate) fn guard_status<F: FnOnce() -> OtelStatus>(f: F) -> OtelStatus {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(s) => s,
        Err(_) => {
            api_ffi::set_last_error("caught panic at FFI boundary");
            OtelStatus::InternalError
        }
    }
}

pub(crate) fn guard_ptr<T, F: FnOnce() -> *mut T>(f: F) -> *mut T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(p) => p,
        Err(_) => {
            api_ffi::set_last_error("caught panic at FFI boundary");
            std::ptr::null_mut()
        }
    }
}

pub(crate) fn guard_unit<F: FnOnce()>(f: F) {
    let _ = catch_unwind(AssertUnwindSafe(f));
}
