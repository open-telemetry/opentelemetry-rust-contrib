//! Shared C-ABI definitions for the `opentelemetry-c` API/SDK split.
//!
//! This crate is an **internal** rlib linked statically into both the `opentelemetry-c-api`
//! and `opentelemetry-c-sdk` cdylibs. It deliberately contains:
//!
//! - only `#[repr(C)]` value types and the internal implementation [`OtelImplVtable`], and
//! - pure helper functions (no `#[no_mangle]` exports, no `static` global state),
//!
//! so that linking it into both libraries introduces no duplicate exported symbols and no
//! duplicate global provider state. The **single** global provider slot lives in the API
//! cdylib; the SDK registers into it across the C ABI (see the API crate).
//!
//! Fallible conversions return [`AbiError`] (a status plus a `'static` message) rather than
//! touching an error slot, because the thread-local diagnostic slot is owned by whichever
//! cdylib is calling; the caller records the message in its own slot.

#![allow(clippy::missing_safety_doc)]

use std::os::raw::{c_char, c_void};

/// Status code returned by fallible C API functions. Mirrors `otel_status_t`.
///
/// `Ok` (0) is success; any non-zero value is a failure. New variants may be appended.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtelStatus {
    /// Operation completed successfully.
    Ok = 0,
    /// A required pointer argument was NULL, or a handle failed validation.
    InvalidArgument = 1,
    /// A string argument was not valid UTF-8 where UTF-8 is required.
    InvalidUtf8 = 2,
    /// Configuration supplied to the SDK builder was invalid.
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

/// A fallible-conversion error: a status code and a `'static` diagnostic message. The
/// caller records the message in its own thread-local error slot.
#[derive(Debug, Clone, Copy)]
pub struct AbiError {
    /// The status to return to C.
    pub status: OtelStatus,
    /// A human-readable diagnostic, recorded by the caller.
    pub message: &'static str,
}

impl AbiError {
    const fn new(status: OtelStatus, message: &'static str) -> Self {
        AbiError { status, message }
    }
}

/// A C boolean, passed across the FFI boundary as a raw 32-bit integer (`0` = false,
/// non-zero = true). An integer alias (not an enum) so every bit pattern is well-defined.
pub type OtelBool = u32;

/// A borrowed, length-delimited UTF-8 string (mirrors `otel_string_view_t`).
///
/// `ptr` may be NULL only when `len == 0`. The bytes need not be NUL-terminated and must
/// remain valid for the duration of the call.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct OtelStringView {
    /// Pointer to the first UTF-8 byte, or NULL when `len == 0`.
    pub ptr: *const c_char,
    /// Number of bytes referenced by `ptr`.
    pub len: usize,
}

impl OtelStringView {
    /// An empty view (NULL pointer, zero length).
    pub fn empty() -> Self {
        OtelStringView {
            ptr: std::ptr::null(),
            len: 0,
        }
    }

    /// Borrow the raw bytes. Rejects a NULL pointer with non-zero length, or a length
    /// exceeding `isize::MAX` (a `slice::from_raw_parts` safety precondition), before any
    /// slice is formed.
    ///
    /// # Safety
    /// `ptr` must be valid for reads of `len` bytes, or NULL when `len == 0`.
    unsafe fn as_bytes(&self) -> Result<&[u8], AbiError> {
        if self.len == 0 {
            return Ok(&[]);
        }
        if self.ptr.is_null() {
            return Err(AbiError::new(
                OtelStatus::InvalidArgument,
                "string view has NULL ptr with non-zero len",
            ));
        }
        if self.len > isize::MAX as usize {
            return Err(AbiError::new(
                OtelStatus::InvalidArgument,
                "string view len exceeds the maximum supported size",
            ));
        }
        // SAFETY: non-NULL, caller guarantees `len` valid bytes, `len <= isize::MAX`.
        Ok(unsafe { std::slice::from_raw_parts(self.ptr.cast::<u8>(), self.len) })
    }

    /// Convert to a `&str`, requiring valid UTF-8.
    ///
    /// # Safety
    /// `ptr` must be valid for reads of `len` bytes, or NULL when `len == 0`.
    pub unsafe fn as_str(&self) -> Result<&str, AbiError> {
        // SAFETY: forwarded to the caller's contract.
        let bytes = unsafe { self.as_bytes() }?;
        std::str::from_utf8(bytes)
            .map_err(|_| AbiError::new(OtelStatus::InvalidUtf8, "string view is not valid UTF-8"))
    }

    /// Convert to an owned `String`, requiring valid UTF-8. The copy is fallible.
    ///
    /// # Safety
    /// `ptr` must be valid for reads of `len` bytes, or NULL when `len == 0`.
    pub unsafe fn to_string_strict(self) -> Result<String, AbiError> {
        // SAFETY: forwarded to the caller's contract.
        let s = unsafe { self.as_str() }?;
        try_owned_string(s)
    }

    /// Convert to an owned `String`, replacing invalid UTF-8 with U+FFFD. The copy is
    /// fallible (allocation failure yields `InternalError` instead of aborting).
    ///
    /// # Safety
    /// `ptr` must be valid for reads of `len` bytes, or NULL when `len == 0`.
    pub unsafe fn to_string_lossy(self) -> Result<String, AbiError> {
        // SAFETY: forwarded to the caller's contract.
        let bytes = unsafe { self.as_bytes() }?;
        try_owned_string_lossy(bytes)
    }
}

fn alloc_err() -> AbiError {
    AbiError::new(OtelStatus::InternalError, "failed to allocate string")
}

fn push_str_fallible(out: &mut String, s: &str) -> Result<(), AbiError> {
    out.try_reserve(s.len()).map_err(|_| alloc_err())?;
    out.push_str(s);
    Ok(())
}

/// Fallibly copy a `&str` into an owned `String`.
pub fn try_owned_string(s: &str) -> Result<String, AbiError> {
    let mut owned = String::new();
    push_str_fallible(&mut owned, s)?;
    Ok(owned)
}

/// Fallibly build the lossy (U+FFFD-replaced) owned form of `bytes`, matching
/// `String::from_utf8_lossy` but never allocating infallibly from C-controlled length.
pub fn try_owned_string_lossy(bytes: &[u8]) -> Result<String, AbiError> {
    let mut out = String::new();
    let mut rest = bytes;
    loop {
        match std::str::from_utf8(rest) {
            Ok(valid) => {
                push_str_fallible(&mut out, valid)?;
                return Ok(out);
            }
            Err(err) => {
                let good = err.valid_up_to();
                // SAFETY: `rest[..good]` is valid UTF-8 by the definition of `valid_up_to`.
                let valid = unsafe { std::str::from_utf8_unchecked(&rest[..good]) };
                push_str_fallible(&mut out, valid)?;
                push_str_fallible(&mut out, "\u{FFFD}")?;
                match err.error_len() {
                    Some(bad) => rest = &rest[good + bad..],
                    None => return Ok(out),
                }
            }
        }
    }
}

/// Discriminant identifying the active member of [`OtelAttributeValue`]. Crosses the ABI
/// as a raw `u32`; validated via [`OtelAttributeType::from_u32`].
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtelAttributeType {
    /// `string_value` is active.
    String = 0,
    /// `bool_value` is active.
    Bool = 1,
    /// `int64_value` is active.
    Int64 = 2,
    /// `double_value` is active.
    Double = 3,
}

impl OtelAttributeType {
    /// Validate a raw tag, returning `None` for unknown values.
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(OtelAttributeType::String),
            1 => Some(OtelAttributeType::Bool),
            2 => Some(OtelAttributeType::Int64),
            3 => Some(OtelAttributeType::Double),
            _ => None,
        }
    }
}

/// Tagged-union payload for an attribute value.
#[repr(C)]
#[derive(Clone, Copy)]
pub union OtelAttributeValue {
    /// Active for [`OtelAttributeType::String`].
    pub string_value: OtelStringView,
    /// Active for [`OtelAttributeType::Bool`] (`0` = false, else true).
    pub bool_value: OtelBool,
    /// Active for [`OtelAttributeType::Int64`].
    pub int64_value: i64,
    /// Active for [`OtelAttributeType::Double`].
    pub double_value: f64,
}

/// A single typed attribute: a key plus a tagged value (mirrors `otel_key_value_t`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct OtelKeyValue {
    /// Attribute key (UTF-8). Must not be empty.
    pub key: OtelStringView,
    /// Selects the active member of `value`; must be a valid [`OtelAttributeType`] tag.
    pub value_type: u32,
    /// The attribute value payload.
    pub value: OtelAttributeValue,
}

/// Span kind, mirroring `opentelemetry::trace::SpanKind`. Crosses the ABI as a raw `u32`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtelSpanKind {
    /// Internal operation with no remote counterpart (default).
    Internal = 0,
    /// Handles a synchronous inbound request.
    Server = 1,
    /// Issues a synchronous outbound request.
    Client = 2,
    /// Initiates an asynchronous message.
    Producer = 3,
    /// Processes an asynchronous message.
    Consumer = 4,
}

impl OtelSpanKind {
    /// Validate a raw span-kind value, returning `None` if unknown.
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(OtelSpanKind::Internal),
            1 => Some(OtelSpanKind::Server),
            2 => Some(OtelSpanKind::Client),
            3 => Some(OtelSpanKind::Producer),
            4 => Some(OtelSpanKind::Consumer),
            _ => None,
        }
    }
}

/// Span status code, mirroring `opentelemetry::trace::Status`. Crosses the ABI as `u32`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtelSpanStatusCode {
    /// Default, unset status.
    Unset = 0,
    /// Explicitly successful.
    Ok = 1,
    /// The operation contains an error.
    Error = 2,
}

impl OtelSpanStatusCode {
    /// Validate a raw status-code value, returning `None` if unknown.
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(OtelSpanStatusCode::Unset),
            1 => Some(OtelSpanStatusCode::Ok),
            2 => Some(OtelSpanStatusCode::Error),
            _ => None,
        }
    }
}

/// Internal implementation vtable registered by the SDK into the API-owned global slot
/// (and returned from `otel_sdk_get_tracer_provider`).
///
/// Every function is `extern "C"` and takes an opaque `*mut c_void` context that the SDK
/// allocated and only the SDK frees (via the `*_free` entries). No Rust types cross this
/// boundary. A single `'static` instance of this vtable lives in the SDK cdylib; the API
/// stores a `*const OtelImplVtable` in each handle it creates for an SDK-backed object.
///
/// ## Provider context ownership (reference counting)
///
/// A `provider_ctx` is **reference-counted**. Each `provider_ctx` value is one owned
/// reference that must be released with exactly one [`provider_free`](Self::provider_free).
/// [`provider_retain`](Self::provider_retain) produces a **new, independent** owned
/// reference to the same underlying provider (a cheap `Arc` clone in the SDK). This lets
/// the API resolve the process-global provider without a use-after-free: it retains the
/// slot's context *while holding the global read lock* (so a concurrent
/// `register`/replace — which needs the write lock — cannot free it mid-retain), releases
/// the lock, uses the retained reference, then frees it. A replacement frees only the
/// slot's own reference; retained references keep the provider alive independently.
///
/// Tracer and span contexts are single-owner (not reference-counted): each is created by
/// one call and freed by exactly one matching `*_free`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct OtelImplVtable {
    /// Create a tracer from a provider context. Returns an opaque tracer context.
    pub provider_get_tracer: extern "C" fn(
        provider_ctx: *mut c_void,
        name: OtelStringView,
        version: OtelStringView,
        schema_url: OtelStringView,
    ) -> *mut c_void,
    /// Produce a **new owned reference** to the same provider (an `Arc` clone in the SDK),
    /// or NULL on failure. The returned context must be released with one `provider_free`.
    /// Used by the API to safely snapshot the global provider across a lock boundary.
    pub provider_retain: extern "C" fn(provider_ctx: *mut c_void) -> *mut c_void,
    /// Release one owned reference to a provider context (see the ownership notes above).
    pub provider_free: extern "C" fn(provider_ctx: *mut c_void),

    /// Start a span from a tracer context. `parent_span_ctx` is NULL for a root span, or
    /// a span context previously produced by *this same vtable*. Returns a span context.
    pub tracer_start_span: extern "C" fn(
        tracer_ctx: *mut c_void,
        name: OtelStringView,
        kind: u32,
        parent_span_ctx: *mut c_void,
    ) -> *mut c_void,
    /// Free a tracer context.
    pub tracer_free: extern "C" fn(tracer_ctx: *mut c_void),

    /// Set a string attribute. Returns a status (e.g. invalid key/UTF-8/allocation).
    pub span_set_string: extern "C" fn(
        span_ctx: *mut c_void,
        key: OtelStringView,
        value: OtelStringView,
    ) -> OtelStatus,
    /// Set a boolean attribute (`0` = false, non-zero = true).
    pub span_set_bool:
        extern "C" fn(span_ctx: *mut c_void, key: OtelStringView, value: OtelBool) -> OtelStatus,
    /// Set an i64 attribute.
    pub span_set_i64:
        extern "C" fn(span_ctx: *mut c_void, key: OtelStringView, value: i64) -> OtelStatus,
    /// Set an f64 attribute.
    pub span_set_f64:
        extern "C" fn(span_ctx: *mut c_void, key: OtelStringView, value: f64) -> OtelStatus,
    /// Add a timestamped event with optional attributes.
    pub span_add_event: extern "C" fn(
        span_ctx: *mut c_void,
        name: OtelStringView,
        attributes: *const OtelKeyValue,
        attribute_count: usize,
    ) -> OtelStatus,
    /// Set the span status (`code` is an `OtelSpanStatusCode` value).
    pub span_set_status:
        extern "C" fn(span_ctx: *mut c_void, code: u32, description: OtelStringView) -> OtelStatus,
    /// Rename the span.
    pub span_update_name: extern "C" fn(span_ctx: *mut c_void, name: OtelStringView) -> OtelStatus,
    /// End the span. The API guards idempotency (calls this at most once per span).
    pub span_end: extern "C" fn(span_ctx: *mut c_void),
    /// Free a span context. The API always ends the span (via `span_end`) before calling
    /// this; dropping the underlying span additionally ends it if it was somehow not ended,
    /// so no span is left unfinished (and no span is double-ended).
    pub span_free: extern "C" fn(span_ctx: *mut c_void),
}

// Compile-time ABI guards (64-bit) mirroring the C header `_Static_assert`s.
#[cfg(target_pointer_width = "64")]
const _: () = {
    use std::mem::{align_of, size_of};
    assert!(size_of::<OtelStringView>() == 16);
    assert!(align_of::<OtelStringView>() == 8);
    assert!(size_of::<OtelAttributeValue>() == 16);
    assert!(size_of::<OtelKeyValue>() == 40);
    assert!(align_of::<OtelKeyValue>() == 8);
};

/// Assert (mostly for documentation) that a raw `void*` round-trips; used by the API/SDK.
pub fn null_mut() -> *mut c_void {
    std::ptr::null_mut()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(s: &str) -> OtelStringView {
        OtelStringView {
            ptr: s.as_ptr().cast::<c_char>(),
            len: s.len(),
        }
    }

    #[test]
    fn string_helpers_roundtrip_and_guard() {
        assert_eq!(unsafe { sv("hi").as_str() }.unwrap(), "hi");
        assert_eq!(unsafe { sv("hi").to_string_strict() }.unwrap(), "hi");
        // oversized len rejected before slice creation
        let dangling = std::ptr::NonNull::<c_char>::dangling().as_ptr() as *const c_char;
        let big = OtelStringView {
            ptr: dangling,
            len: (isize::MAX as usize) + 1,
        };
        assert_eq!(
            unsafe { big.as_str() }.unwrap_err().status,
            OtelStatus::InvalidArgument
        );
    }

    #[test]
    fn lossy_matches_std() {
        let raw = b"a\xffb".as_slice();
        assert_eq!(
            try_owned_string_lossy(raw).unwrap(),
            String::from_utf8_lossy(raw)
        );
    }

    #[test]
    fn discriminant_validation() {
        assert_eq!(
            OtelAttributeType::from_u32(3),
            Some(OtelAttributeType::Double)
        );
        assert_eq!(OtelAttributeType::from_u32(9), None);
        assert_eq!(OtelSpanKind::from_u32(2), Some(OtelSpanKind::Client));
        assert_eq!(OtelSpanKind::from_u32(9), None);
        assert_eq!(
            OtelSpanStatusCode::from_u32(2),
            Some(OtelSpanStatusCode::Error)
        );
        assert_eq!(OtelSpanStatusCode::from_u32(9), None);
    }
}
