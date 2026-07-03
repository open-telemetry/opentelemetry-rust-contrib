//! Opaque handle plumbing: magic-number validation, allocation helpers, and
//! panic-catching wrappers used by every FFI entry point in the API crate.
//!
//! ## What the magic value can and cannot do
//!
//! The magic check is a **best-effort diagnostic, not a memory-safety boundary**. Callers
//! must pass NULL or a live handle of the **exact expected type** returned by this library.
//! [`checked_ref`] rejects NULL up front, but to read the magic it must
//! first dereference the pointer *as the expected type*, so the check is only meaningful
//! once that contract holds. Passing a wrong handle type, a freed handle, or a foreign
//! pointer, double-destroying, or racing `destroy` with any other call is undefined
//! behavior; the magic cannot be relied upon to catch it.

use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::error::{set_last_error, OtelStatus};

/// A heap-allocated handle carrying a validation magic number.
pub(crate) trait HasMagic {
    /// Unique per-type magic value.
    const MAGIC: u64;
    /// Returns the stored magic value.
    fn magic(&self) -> u64;
    /// Overwrites the stored magic (used to poison freed handles).
    fn set_magic(&mut self, value: u64);
}

/// Borrow a `&T` from a `*const T` handle after NULL and (best-effort) magic validation.
///
/// # Safety
/// `ptr` must be NULL or point to a live handle of the exact type `T` produced by this
/// library, not destroyed or mutated concurrently for the borrow.
pub(crate) unsafe fn checked_ref<'a, T: HasMagic>(ptr: *const T) -> Option<&'a T> {
    if ptr.is_null() {
        set_last_error("null handle passed to OpenTelemetry C API");
        return None;
    }
    // SAFETY: caller guarantees a live `T` (or NULL, handled above).
    let handle = unsafe { &*ptr };
    if handle.magic() != T::MAGIC {
        set_last_error("handle failed validation: not a live handle of the expected type");
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
/// NULL is ignored; a magic mismatch leaves the allocation untouched. Best-effort only:
/// use-after-destroy, double-destroy, or racing `destroy` is undefined behavior.
///
/// # Safety
/// `ptr` must be NULL or a pointer returned by [`into_raw`] for the same `T` that has not
/// been destroyed, and must not be used or destroyed concurrently.
pub(crate) unsafe fn destroy<T: HasMagic>(ptr: *mut T) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: caller guarantees a live allocation for `T`.
    let handle = unsafe { &mut *ptr };
    if handle.magic() != T::MAGIC {
        return;
    }
    handle.set_magic(0);
    // SAFETY: validated above; reconstitute the Box to run drop + free.
    drop(unsafe { Box::from_raw(ptr) });
}

/// Run `f`, converting any Rust panic into [`OtelStatus::InternalError`].
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
pub(crate) fn guard_ptr<T, F: FnOnce() -> *mut T>(f: F) -> *mut T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(ptr) => ptr,
        Err(_) => {
            set_last_error("caught panic at FFI boundary");
            std::ptr::null_mut()
        }
    }
}

/// Run `f`, swallowing any Rust panic (for `void` destructors/setters).
pub(crate) fn guard_unit<F: FnOnce()>(f: F) {
    let _ = catch_unwind(AssertUnwindSafe(f));
}

/// Run `f`, returning `fallback` if it panics (for plain-value getters).
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
        const MAGIC: u64 = 0xABCD;
        fn magic(&self) -> u64 {
            self.magic
        }
        fn set_magic(&mut self, v: u64) {
            self.magic = v;
        }
    }

    #[test]
    fn ref_rejects_null_and_bad_magic_then_roundtrips() {
        assert!(unsafe { checked_ref::<Dummy>(std::ptr::null()) }.is_none());
        let bad = into_raw(Dummy { magic: 0, value: 1 });
        assert!(unsafe { checked_ref(bad) }.is_none());
        unsafe { drop(Box::from_raw(bad)) };

        let ptr = into_raw(Dummy {
            magic: Dummy::MAGIC,
            value: 7,
        });
        assert_eq!(unsafe { checked_ref(ptr) }.unwrap().value, 7);
        unsafe { destroy(ptr) };
    }

    #[test]
    fn guards_catch_panics() {
        assert_eq!(guard_status(|| panic!("x")), OtelStatus::InternalError);
        assert!(guard_ptr::<u8, _>(|| panic!("x")).is_null());
        guard_unit(|| panic!("x"));
        assert_eq!(guard_value(5, || panic!("x")), 5);
    }
}
