//! Opaque handle plumbing: magic-number validation, allocation helpers, and
//! panic-catching wrappers used by every FFI entry point.
//!
//! All handles are heap-allocated `Box`es whose raw pointers are handed to C. Each
//! handle stores a distinct 64-bit magic value so that obviously-invalid or
//! type-confused pointers can be rejected instead of dereferenced blindly.

use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::error::{set_last_error, OtelStatus};

/// A heap-allocated handle that carries a validation magic number.
///
/// Each concrete handle type picks a unique [`MAGIC`](HasMagic::MAGIC) constant.
pub(crate) trait HasMagic {
    /// Unique per-type magic value used to validate raw pointers.
    const MAGIC: u64;
    /// Returns the magic value currently stored in the handle.
    fn magic(&self) -> u64;
    /// Overwrites the stored magic value (used to poison freed handles).
    fn set_magic(&mut self, value: u64);
}

/// Borrow a `&T` from a `*const T` handle after null and magic validation.
///
/// Returns `None` (and records a diagnostic) if the pointer is NULL or the magic
/// value does not match.
///
/// # Safety
/// `ptr` must either be NULL or point to a live `T` produced by this library.
pub(crate) unsafe fn checked_ref<'a, T: HasMagic>(ptr: *const T) -> Option<&'a T> {
    if ptr.is_null() {
        set_last_error("null handle passed to OpenTelemetry C API");
        return None;
    }
    // SAFETY: caller guarantees `ptr` is either NULL (handled above) or valid.
    let handle = unsafe { &*ptr };
    if handle.magic() != T::MAGIC {
        set_last_error("invalid or already-destroyed handle passed to OpenTelemetry C API");
        return None;
    }
    Some(handle)
}

/// Borrow a `&mut T` from a `*mut T` handle after null and magic validation.
///
/// # Safety
/// `ptr` must either be NULL or point to a live, uniquely-borrowed `T` produced by
/// this library.
pub(crate) unsafe fn checked_mut<'a, T: HasMagic>(ptr: *mut T) -> Option<&'a mut T> {
    if ptr.is_null() {
        set_last_error("null handle passed to OpenTelemetry C API");
        return None;
    }
    // SAFETY: caller guarantees `ptr` is either NULL (handled above) or valid.
    let handle = unsafe { &mut *ptr };
    if handle.magic() != T::MAGIC {
        set_last_error("invalid or already-destroyed handle passed to OpenTelemetry C API");
        return None;
    }
    Some(handle)
}

/// Allocate a handle on the heap and return an owning raw pointer for C.
pub(crate) fn into_raw<T>(value: T) -> *mut T {
    Box::into_raw(Box::new(value))
}

/// Reclaim and drop a handle previously created with [`into_raw`].
///
/// NULL is ignored. If the magic value does not match, the pointer is left untouched
/// (this catches the common "wrong handle type" mistake). The magic is poisoned to
/// zero before the box is dropped so that use-after-free is more likely to be caught.
///
/// # Safety
/// `ptr` must either be NULL or a pointer returned by [`into_raw`] for the same `T`
/// that has not already been destroyed.
pub(crate) unsafe fn destroy<T: HasMagic>(ptr: *mut T) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: caller guarantees `ptr` is a live allocation for `T`.
    let handle = unsafe { &mut *ptr };
    if handle.magic() != T::MAGIC {
        // Not one of ours (or already destroyed); do nothing rather than risk UB.
        return;
    }
    handle.set_magic(0);
    // SAFETY: validated above; reconstitute the Box to run drop + free.
    drop(unsafe { Box::from_raw(ptr) });
}

/// Run `f`, converting any Rust panic into [`OtelStatus::InternalError`].
///
/// This is the panic firewall for entry points that return a status code. Raw pointer
/// captures are not `UnwindSafe`, so [`AssertUnwindSafe`] is used deliberately; on a
/// caught panic we only report an error and never touch partially-modified state.
pub(crate) fn guard_status<F: FnOnce() -> OtelStatus>(f: F) -> OtelStatus {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(status) => status,
        Err(_) => {
            set_last_error("caught panic at FFI boundary");
            OtelStatus::InternalError
        }
    }
}

/// Run `f`, converting any Rust panic into a NULL pointer.
///
/// Panic firewall for entry points that return an owning handle pointer.
pub(crate) fn guard_ptr<T, F: FnOnce() -> *mut T>(f: F) -> *mut T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(ptr) => ptr,
        Err(_) => {
            set_last_error("caught panic at FFI boundary");
            std::ptr::null_mut()
        }
    }
}

/// Run `f`, swallowing any Rust panic (used by `void` destructors and setters that
/// cannot return a status).
pub(crate) fn guard_unit<F: FnOnce()>(f: F) {
    let _ = catch_unwind(AssertUnwindSafe(f));
}

/// Run `f`, returning `fallback` if it panics.
///
/// Panic firewall for entry points that return a plain value with an obvious safe
/// default and must not record an error (avoids re-entrancy with the error slot).
pub(crate) fn guard_value<T, F: FnOnce() -> T>(fallback: T, f: F) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dummy {
        magic: u64,
        value: i32,
    }

    impl HasMagic for Dummy {
        const MAGIC: u64 = 0x0102_0304_0506_0708;
        fn magic(&self) -> u64 {
            self.magic
        }
        fn set_magic(&mut self, value: u64) {
            self.magic = value;
        }
    }

    fn new_dummy(value: i32) -> Dummy {
        Dummy {
            magic: Dummy::MAGIC,
            value,
        }
    }

    #[test]
    fn checked_ref_rejects_null() {
        assert!(unsafe { checked_ref::<Dummy>(std::ptr::null()) }.is_none());
    }

    #[test]
    fn checked_ref_rejects_bad_magic() {
        let bad = Dummy { magic: 0, value: 1 };
        let ptr: *const Dummy = &bad;
        assert!(unsafe { checked_ref(ptr) }.is_none());
    }

    #[test]
    fn into_raw_checked_destroy_roundtrip() {
        let ptr = into_raw(new_dummy(7));
        assert_eq!(unsafe { checked_ref(ptr) }.unwrap().value, 7);
        unsafe { checked_mut(ptr) }.unwrap().value = 9;
        assert_eq!(unsafe { checked_ref(ptr) }.unwrap().value, 9);
        unsafe { destroy(ptr) };
    }

    #[test]
    fn destroy_null_is_noop() {
        unsafe { destroy::<Dummy>(std::ptr::null_mut()) };
    }

    #[test]
    fn destroy_wrong_magic_is_ignored() {
        // A stack value with the wrong magic must not be freed by `destroy`.
        let mut not_ours = Dummy { magic: 0, value: 1 };
        let ptr: *mut Dummy = &mut not_ours;
        unsafe { destroy(ptr) };
        // Still usable; nothing was freed.
        assert_eq!(not_ours.value, 1);
    }

    #[test]
    fn guard_status_catches_panic() {
        let status = guard_status(|| panic!("boom"));
        assert_eq!(status, OtelStatus::InternalError);
        let ok = guard_status(|| OtelStatus::Ok);
        assert_eq!(ok, OtelStatus::Ok);
    }

    #[test]
    fn guard_ptr_catches_panic() {
        let ptr: *mut Dummy = guard_ptr(|| panic!("boom"));
        assert!(ptr.is_null());
    }

    #[test]
    fn guard_unit_catches_panic() {
        guard_unit(|| panic!("boom"));
    }
}
