//! Bridge to the API cdylib's internal registration ABI.
//!
//! In a normal build these are `extern "C"` imports resolved at load time against
//! `libopentelemetry_c_api` (see `build.rs`). Under `cfg(test)` — where the SDK rlib is
//! linked into a test binary that does *not* load the API cdylib — they are replaced with
//! in-process stubs so the SDK's own unit tests link and can observe registration. The
//! true cross-artifact behavior is proven by the separate C link/run test.

use std::os::raw::{c_char, c_void};

use opentelemetry_c_abi::{OtelImplVtable, OtelStatus};

#[cfg(not(test))]
mod imp {
    use super::*;
    // MSRV is 1.75; `unsafe extern` blocks require 1.82. Keep a plain extern block and
    // allow the Rust-2024-compat lint. These import the API cdylib's internal symbols.
    #[allow(missing_unsafe_on_extern)]
    extern "C" {
        pub fn otel_api_register_global_provider(
            vtable: *const OtelImplVtable,
            provider_ctx: *mut c_void,
        ) -> OtelStatus;
        pub fn otel_api_provider_new(
            vtable: *const OtelImplVtable,
            provider_ctx: *mut c_void,
        ) -> *mut c_void;
        pub fn otel_api_set_last_error(ptr: *const c_char, len: usize);
        pub fn otel_api_clear_last_error();
    }
}

#[cfg(test)]
mod imp {
    use super::*;
    use std::cell::RefCell;
    use std::sync::Mutex;

    thread_local! {
        pub(super) static LAST_ERROR: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }
    // Records the most recently registered (vtable, ctx) so tests can drive it.
    pub(super) static REGISTERED: Mutex<Option<(usize, usize)>> = Mutex::new(None);

    /// # Safety
    /// Test stub mirroring the real ABI.
    pub unsafe fn otel_api_register_global_provider(
        vtable: *const OtelImplVtable,
        provider_ctx: *mut c_void,
    ) -> OtelStatus {
        *REGISTERED.lock().unwrap() = Some((vtable as usize, provider_ctx as usize));
        OtelStatus::Ok
    }
    /// # Safety
    /// Test stub mirroring the real ABI.
    pub unsafe fn otel_api_provider_new(
        _vtable: *const OtelImplVtable,
        provider_ctx: *mut c_void,
    ) -> *mut c_void {
        provider_ctx
    }
    /// # Safety
    /// Test stub mirroring the real ABI.
    pub unsafe fn otel_api_set_last_error(ptr: *const c_char, len: usize) {
        LAST_ERROR.with(|slot| {
            let mut b = slot.borrow_mut();
            b.clear();
            if !ptr.is_null() && len > 0 && len <= isize::MAX as usize {
                b.extend_from_slice(unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) });
            }
        });
    }
    /// # Safety
    /// Test stub mirroring the real ABI.
    pub unsafe fn otel_api_clear_last_error() {
        LAST_ERROR.with(|slot| slot.borrow_mut().clear());
    }
}

/// Install `vtable`/`provider_ctx` as the process-global provider (API-owned slot).
pub(crate) fn register_global_provider(
    vtable: *const OtelImplVtable,
    provider_ctx: *mut c_void,
) -> OtelStatus {
    unsafe { imp::otel_api_register_global_provider(vtable, provider_ctx) }
}

/// Wrap an SDK provider context in an owned API `otel_tracer_provider_t` handle.
pub(crate) fn provider_new(
    vtable: *const OtelImplVtable,
    provider_ctx: *mut c_void,
) -> *mut c_void {
    unsafe { imp::otel_api_provider_new(vtable, provider_ctx) }
}

/// Record a diagnostic in the API-owned thread-local error slot.
pub(crate) fn set_last_error(message: &str) {
    unsafe { imp::otel_api_set_last_error(message.as_ptr().cast::<c_char>(), message.len()) };
}

/// Clear the API-owned thread-local error slot.
pub(crate) fn clear_last_error() {
    unsafe { imp::otel_api_clear_last_error() };
}

#[cfg(test)]
pub(crate) mod test_probe {
    use super::*;

    // Only the OTLP-backed `set_as_global` unit test drives this probe; without the `otlp`
    // feature that test is compiled out, so gate the helper to match (avoids dead_code).
    #[cfg(feature = "otlp")]
    pub fn registered() -> Option<(*const OtelImplVtable, *mut c_void)> {
        imp::REGISTERED
            .lock()
            .unwrap()
            .as_ref()
            .map(|&(v, c)| (v as *const OtelImplVtable, c as *mut c_void))
    }

    /// The current thread's recorded last-error message (empty if none).
    pub fn last_error() -> String {
        imp::LAST_ERROR.with(|slot| String::from_utf8_lossy(&slot.borrow()).into_owned())
    }
}
