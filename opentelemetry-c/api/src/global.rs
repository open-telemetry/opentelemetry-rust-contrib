//! The **API-owned global provider slot** and the internal registration ABI.
//!
//! This is the heart of the split. The single global provider slot lives in *this* (API)
//! cdylib. The separate SDK cdylib installs itself by calling the exported
//! [`otel_api_register_global_provider`] across the C ABI, passing a `#[repr(C)]`
//! implementation vtable plus an opaque provider context. There is exactly one global
//! slot in the process (owned here), so an API-only instrumentation library and the
//! application observe the same installed provider.

use std::os::raw::c_void;
use std::sync::RwLock;

use opentelemetry_c_abi::OtelImplVtable;

use crate::error::{clear_last_error, fail, has_last_error, set_last_error, OtelStatus};
use crate::handle::{guard_ptr, guard_status, into_raw};
use crate::trace::{OtelTracerProvider, ProviderInner};

/// The registered implementation, or the no-op default (`vtable` NULL).
struct GlobalProvider {
    vtable: *const OtelImplVtable,
    ctx: *mut c_void,
}

// SAFETY: the slot only ever holds raw pointers into the SDK cdylib. The registered
// provider context is documented to be safe for shared, concurrent use (the SDK's
// provider is `Arc`-backed and its vtable functions take `&`), so sharing the slot across
// threads is sound. The API never dereferences these as Rust types — only via the vtable.
unsafe impl Send for GlobalProvider {}
unsafe impl Sync for GlobalProvider {}

static GLOBAL: RwLock<GlobalProvider> = RwLock::new(GlobalProvider {
    vtable: std::ptr::null(),
    ctx: std::ptr::null_mut(),
});

/// Outcome of attempting to retain the currently-installed global provider.
///
/// This deliberately distinguishes "no provider installed" (a legitimate no-op) from
/// "a provider is installed but retaining it failed" (a real error). Collapsing both into a
/// NULL context would turn a backed failure into a success-shaped no-op handle.
pub(crate) enum GlobalRetain {
    /// No SDK/provider is installed — callers should behave as an unbacked no-op.
    NoProvider,
    /// A provider was installed and its context retained. The caller owns `ctx` and must
    /// release it with exactly one `vtable.provider_free(ctx)`.
    Retained {
        vtable: *const OtelImplVtable,
        ctx: *mut c_void,
    },
    /// A provider is installed but retaining its context failed. The last-error slot is set
    /// (either by the vtable or a default message); callers must surface this as an error,
    /// not a no-op.
    RetainFailed,
}

/// Retain the currently-installed global provider **under the read lock**.
///
/// Returns [`GlobalRetain::NoProvider`] when no SDK is installed, [`GlobalRetain::Retained`]
/// with an **owned** provider context the caller must release via `vtable.provider_free`, or
/// [`GlobalRetain::RetainFailed`] when a provider is installed but `provider_retain` returned
/// NULL (in which case the last-error slot is left set).
///
/// Retaining while holding the read lock is what eliminates the use-after-free: a
/// concurrent `register`/replacement must take the *write* lock, so it cannot free the
/// slot's context during the retain. The returned reference is independent of the slot, so
/// it remains valid even if the slot is replaced (and its old reference freed) immediately
/// afterwards.
pub(crate) fn retain_global() -> GlobalRetain {
    let g = GLOBAL.read().unwrap_or_else(|p| p.into_inner());
    if g.vtable.is_null() {
        return GlobalRetain::NoProvider;
    }
    // SAFETY: `g.vtable` is a live registered vtable and `g.ctx` is alive for the duration
    // of this read-locked scope (replacement needs the write lock). `provider_retain`
    // produces a new owned reference or NULL.
    let retained = unsafe { ((*g.vtable).provider_retain)(g.ctx) };
    if retained.is_null() {
        // A provider IS installed but retaining it failed. This is an error, not a no-op.
        // Preserve any message the vtable recorded; otherwise record a default so the caller
        // always has a diagnostic. (`get_tracer` clears the slot before calling us, so a set
        // slot here means the vtable set it.)
        if !has_last_error() {
            set_last_error("global provider retain failed");
        }
        return GlobalRetain::RetainFailed;
    }
    GlobalRetain::Retained {
        vtable: g.vtable,
        ctx: retained,
    }
}

/// **Internal ABI (called by the SDK cdylib).** Install `vtable`/`provider_ctx` as the
/// process-global provider, replacing any previous one (whose context is freed via its own
/// `provider_free`). Returns `OTEL_STATUS_INVALID_ARGUMENT` if `vtable` is NULL.
///
/// There is no deregister/clear entry point: an installed provider stays in the slot until
/// it is replaced by another install or the process exits. Consequently the library that
/// owns `vtable`/`provider_ctx` (the SDK cdylib) must stay loaded for that whole window.
///
/// # Safety
/// `vtable` must point to a `'static` [`OtelImplVtable`] and `provider_ctx` to a provider
/// object owned by the caller; ownership of `provider_ctx` transfers to the API slot.
#[no_mangle]
pub unsafe extern "C" fn otel_api_register_global_provider(
    vtable: *const OtelImplVtable,
    provider_ctx: *mut c_void,
) -> OtelStatus {
    guard_status(|| {
        clear_last_error();
        if vtable.is_null() {
            return fail(
                OtelStatus::InvalidArgument,
                "register_global_provider: vtable must not be NULL",
            );
        }
        // Swap in the new provider, capturing the old one to free outside the lock.
        let old = {
            let mut g = GLOBAL.write().unwrap_or_else(|p| p.into_inner());
            let old = GlobalProvider {
                vtable: g.vtable,
                ctx: g.ctx,
            };
            g.vtable = vtable;
            g.ctx = provider_ctx;
            old
        };
        if !old.vtable.is_null() {
            // SAFETY: `old.vtable` was a valid registered vtable. This releases only the
            // slot's own reference to the old provider (exactly once). Any references
            // retained by concurrent `retain_global` calls are independent and keep the old
            // provider alive until they are freed, so this cannot race into a use-after-free.
            let free = unsafe { (*old.vtable).provider_free };
            free(old.ctx);
        }
        OtelStatus::Ok
    })
}

/// **Internal ABI (called by the SDK cdylib).** Wrap an SDK provider context in an owned
/// API `otel_tracer_provider_t` handle (used by `otel_sdk_get_tracer_provider`). The
/// returned handle owns `provider_ctx` and frees it (via `provider_free`) when destroyed.
///
/// # Safety
/// `vtable` must point to a `'static` [`OtelImplVtable`]; `provider_ctx` to a provider
/// object whose ownership transfers to the returned handle.
#[no_mangle]
pub unsafe extern "C" fn otel_api_provider_new(
    vtable: *const OtelImplVtable,
    provider_ctx: *mut c_void,
) -> *mut OtelTracerProvider {
    guard_ptr(|| {
        clear_last_error();
        if vtable.is_null() {
            fail(
                OtelStatus::InvalidArgument,
                "provider_new: vtable must not be NULL",
            );
            return std::ptr::null_mut();
        }
        into_raw(OtelTracerProvider::new(ProviderInner::Backed {
            vtable,
            ctx: provider_ctx,
        }))
    })
}

/// Return an owned handle to the process-global tracer provider. Never NULL under normal
/// conditions. Tracers obtained from it reflect whichever SDK is installed at the time of
/// the request. Release with `otel_tracer_provider_destroy()`.
#[no_mangle]
pub extern "C" fn otel_global_tracer_provider() -> *mut OtelTracerProvider {
    guard_ptr(|| {
        clear_last_error();
        into_raw(OtelTracerProvider::new(ProviderInner::Global))
    })
}
