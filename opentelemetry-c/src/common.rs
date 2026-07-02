//! Shared C-compatible value types: string views and typed key/value attributes,
//! plus the conversions from those borrowed C types into owned OpenTelemetry types.
//!
//! Discriminant values (attribute type tags, booleans) cross the C boundary as raw
//! fixed-width integers rather than as Rust `#[repr(C)]` enums: constructing a Rust
//! enum from an out-of-range integer is undefined behavior, so a misbehaving C caller
//! must not be able to trigger it. Booleans use the C convention (`0` = false,
//! non-zero = true); type tags are validated against [`OtelAttributeType`].

use std::os::raw::c_char;

use opentelemetry::{Key, KeyValue, StringValue, Value};

use crate::error::{fail, OtelStatus};

/// A C boolean, passed across the FFI boundary as a raw 32-bit integer: `0` is false
/// and any non-zero value is true.
///
/// This is an integer alias (not a Rust `enum`) on purpose: constructing a field-less
/// Rust enum from an out-of-range integer would be undefined behavior, whereas every
/// bit pattern is a valid `u32`. The C header exposes `otel_bool_t` as a matching
/// `uint32_t` typedef with the readable `OTEL_FALSE` / `OTEL_TRUE` constants.
pub type OtelBool = u32;

/// A borrowed, length-delimited UTF-8 string.
///
/// The bytes are **not** required to be NUL-terminated. `ptr` may be NULL only when
/// `len` is zero (representing an empty/absent string). The referenced bytes must
/// remain valid for the duration of the call they are passed to; this library copies
/// any data it needs to retain before returning.
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
    pub(crate) fn empty() -> Self {
        OtelStringView {
            ptr: std::ptr::null(),
            len: 0,
        }
    }

    /// Borrow the raw bytes described by this view.
    ///
    /// Returns `None` when the view is malformed (NULL pointer with non-zero length).
    ///
    /// # Safety
    /// `ptr` must be valid for reads of `len` bytes, or NULL when `len == 0`.
    unsafe fn as_bytes(&self) -> Option<&[u8]> {
        if self.len == 0 {
            return Some(&[]);
        }
        if self.ptr.is_null() {
            return None;
        }
        // SAFETY: caller guarantees `ptr` covers `len` valid bytes.
        Some(unsafe { std::slice::from_raw_parts(self.ptr as *const u8, self.len) })
    }

    /// Convert to a `&str`, requiring valid UTF-8.
    ///
    /// # Safety
    /// See [`OtelStringView::as_bytes`].
    pub(crate) unsafe fn as_str(&self) -> Result<&str, OtelStatus> {
        // SAFETY: forwarded to the caller's contract.
        let bytes = unsafe { self.as_bytes() }.ok_or_else(|| {
            fail(
                OtelStatus::InvalidArgument,
                "string view has NULL ptr with non-zero len",
            )
        })?;
        std::str::from_utf8(bytes)
            .map_err(|_| fail(OtelStatus::InvalidUtf8, "string view is not valid UTF-8"))
    }

    /// Convert to an owned `String`, requiring valid UTF-8.
    ///
    /// # Safety
    /// See [`OtelStringView::as_bytes`].
    pub(crate) unsafe fn to_string_strict(self) -> Result<String, OtelStatus> {
        // SAFETY: forwarded to the caller's contract.
        unsafe { self.as_str() }.map(|s| s.to_owned())
    }

    /// Convert to an owned `String`, replacing invalid UTF-8 with U+FFFD.
    ///
    /// Used for best-effort fields (attribute keys/values, span/event names) where the
    /// OpenTelemetry specification prefers lossy degradation over hard failure.
    ///
    /// # Safety
    /// See [`OtelStringView::as_bytes`].
    pub(crate) unsafe fn to_string_lossy(self) -> Result<String, OtelStatus> {
        // SAFETY: forwarded to the caller's contract.
        let bytes = unsafe { self.as_bytes() }.ok_or_else(|| {
            fail(
                OtelStatus::InvalidArgument,
                "string view has NULL ptr with non-zero len",
            )
        })?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// Discriminant identifying the active member of the [`OtelAttributeValue`] union.
///
/// The tag crosses the C boundary as a raw `u32` (see [`OtelKeyValue::value_type`]);
/// this enum documents the valid values and validates them via [`Self::from_u32`].
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtelAttributeType {
    /// The `string_value` member is active.
    String = 0,
    /// The `bool_value` member is active.
    Bool = 1,
    /// The `int64_value` member is active.
    Int64 = 2,
    /// The `double_value` member is active.
    Double = 3,
}

impl OtelAttributeType {
    /// Validate a raw tag received from C, returning `None` for unknown values.
    pub(crate) fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(OtelAttributeType::String),
            1 => Some(OtelAttributeType::Bool),
            2 => Some(OtelAttributeType::Int64),
            3 => Some(OtelAttributeType::Double),
            _ => None,
        }
    }
}

/// Tagged-union payload for an attribute value. The active member is selected by the
/// companion tag in [`OtelKeyValue::value_type`].
///
/// Note: `bool_value` is a raw `u32` (`0` = false, non-zero = true) rather than an enum
/// so that any bit pattern a C caller writes is a valid Rust value.
#[repr(C)]
#[derive(Clone, Copy)]
pub union OtelAttributeValue {
    /// Active when the tag is [`OtelAttributeType::String`].
    pub string_value: OtelStringView,
    /// Active when the tag is [`OtelAttributeType::Bool`] (`0` = false, else true).
    pub bool_value: OtelBool,
    /// Active when the tag is [`OtelAttributeType::Int64`].
    pub int64_value: i64,
    /// Active when the tag is [`OtelAttributeType::Double`].
    pub double_value: f64,
}

/// A single typed attribute: a key plus a tagged value.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct OtelKeyValue {
    /// Attribute key (UTF-8). Must not be empty.
    pub key: OtelStringView,
    /// Selects the active member of `value`. Must be a valid [`OtelAttributeType`]
    /// value (`0`=string, `1`=bool, `2`=int64, `3`=double); other values are rejected.
    pub value_type: u32,
    /// The attribute value payload.
    pub value: OtelAttributeValue,
}

impl OtelKeyValue {
    /// Convert this borrowed C attribute into an owned [`KeyValue`].
    ///
    /// Keys and string values use lossy UTF-8 conversion. Returns an error status when
    /// the key is empty/malformed or `value_type` is not a known tag.
    ///
    /// # Safety
    /// All string views inside `self` must satisfy the [`OtelStringView`] contract.
    pub(crate) unsafe fn to_key_value(self) -> Result<KeyValue, OtelStatus> {
        // SAFETY: forwarded to the caller's contract.
        let key = unsafe { self.key.to_string_lossy() }?;
        if key.is_empty() {
            return Err(fail(
                OtelStatus::InvalidArgument,
                "attribute key must not be empty",
            ));
        }
        // Validate the tag before touching the union so an out-of-range value can never
        // cause a type-confused read.
        let value_type = OtelAttributeType::from_u32(self.value_type).ok_or_else(|| {
            fail(
                OtelStatus::InvalidArgument,
                "attribute value_type is not a valid OtelAttributeType tag",
            )
        })?;
        let value: Value = match value_type {
            OtelAttributeType::String => {
                // SAFETY: tag guarantees the string member is active.
                let sv = unsafe { self.value.string_value };
                // SAFETY: forwarded to the caller's contract.
                let s = unsafe { sv.to_string_lossy() }?;
                Value::String(StringValue::from(s))
            }
            OtelAttributeType::Bool => {
                // SAFETY: tag guarantees the bool member is active; any u32 is valid.
                Value::Bool(unsafe { self.value.bool_value } != 0)
            }
            OtelAttributeType::Int64 => {
                // SAFETY: tag guarantees the int64 member is active.
                Value::I64(unsafe { self.value.int64_value })
            }
            OtelAttributeType::Double => {
                // SAFETY: tag guarantees the double member is active.
                Value::F64(unsafe { self.value.double_value })
            }
        };
        Ok(KeyValue::new(Key::from(key), value))
    }
}

/// Convert a borrowed C array of attributes into owned [`KeyValue`]s.
///
/// A NULL `attributes` pointer is treated as an empty list when `count == 0`.
///
/// # Safety
/// `attributes` must point to `count` initialized [`OtelKeyValue`]s (or be NULL when
/// `count == 0`), and every contained string view must be valid.
pub(crate) unsafe fn collect_key_values(
    attributes: *const OtelKeyValue,
    count: usize,
) -> Result<Vec<KeyValue>, OtelStatus> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if attributes.is_null() {
        return Err(fail(
            OtelStatus::InvalidArgument,
            "attribute array is NULL with non-zero count",
        ));
    }
    // Guard the `slice::from_raw_parts` safety precondition: the array's total byte
    // length must not overflow and must not exceed `isize::MAX`. A caller passing a
    // bogus/hostile count is rejected here rather than risking undefined behavior.
    let within_bounds = count
        .checked_mul(std::mem::size_of::<OtelKeyValue>())
        .is_some_and(|bytes| bytes <= isize::MAX as usize);
    if !within_bounds {
        return Err(fail(
            OtelStatus::InvalidArgument,
            "attribute count exceeds the maximum supported size",
        ));
    }
    // Reserve capacity fallibly *before* forming the slice, so an unreasonable count
    // returns an error instead of aborting the process on allocation failure, and no
    // oversized slice is ever formed from C-supplied input.
    let mut out: Vec<KeyValue> = Vec::new();
    if out.try_reserve(count).is_err() {
        return Err(fail(
            OtelStatus::InternalError,
            "failed to allocate space for attributes",
        ));
    }
    // SAFETY: `attributes` is non-NULL, the caller guarantees it covers `count`
    // initialized elements, and the total byte length is within `isize::MAX` (checked
    // above).
    let slice = unsafe { std::slice::from_raw_parts(attributes, count) };
    for kv in slice {
        // SAFETY: each element satisfies the OtelKeyValue contract per caller.
        out.push(unsafe { kv.to_key_value() }?);
    }
    Ok(out)
}

// Compile-time ABI guards. These mirror the `_Static_assert`s in the C headers and
// fail the build if the struct layout ever drifts from the documented C ABI.
#[cfg(target_pointer_width = "64")]
const _: () = {
    use std::mem::{align_of, size_of};
    assert!(size_of::<OtelStringView>() == 16);
    assert!(align_of::<OtelStringView>() == 8);
    assert!(size_of::<OtelAttributeValue>() == 16);
    assert!(size_of::<OtelKeyValue>() == 40);
    assert!(align_of::<OtelKeyValue>() == 8);
};

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::Value;
    use std::os::raw::c_char;

    fn sv(s: &str) -> OtelStringView {
        OtelStringView {
            ptr: s.as_ptr() as *const c_char,
            len: s.len(),
        }
    }

    #[test]
    fn empty_view_converts_to_empty_string() {
        let v = OtelStringView::empty();
        assert_eq!(unsafe { v.to_string_strict() }.unwrap(), "");
        assert_eq!(unsafe { v.as_str() }.unwrap(), "");
    }

    #[test]
    fn valid_utf8_roundtrips() {
        let v = sv("hello");
        assert_eq!(unsafe { v.as_str() }.unwrap(), "hello");
        assert_eq!(unsafe { v.to_string_lossy() }.unwrap(), "hello");
    }

    #[test]
    fn invalid_utf8_strict_fails_but_lossy_succeeds() {
        let bytes = [0xffu8, 0xfe, b'a'];
        let v = OtelStringView {
            ptr: bytes.as_ptr() as *const c_char,
            len: bytes.len(),
        };
        assert_eq!(
            unsafe { v.to_string_strict() }.unwrap_err(),
            OtelStatus::InvalidUtf8
        );
        assert!(unsafe { v.to_string_lossy() }.unwrap().ends_with('a'));
    }

    #[test]
    fn null_ptr_with_nonzero_len_is_invalid() {
        let v = OtelStringView {
            ptr: std::ptr::null(),
            len: 3,
        };
        assert_eq!(
            unsafe { v.as_str() }.unwrap_err(),
            OtelStatus::InvalidArgument
        );
    }

    #[test]
    fn key_value_conversions_cover_all_types() {
        let string_kv = OtelKeyValue {
            key: sv("s"),
            value_type: OtelAttributeType::String as u32,
            value: OtelAttributeValue {
                string_value: sv("v"),
            },
        };
        let kv = unsafe { string_kv.to_key_value() }.unwrap();
        assert_eq!(kv.key.as_str(), "s");
        assert!(matches!(kv.value, Value::String(ref s) if s.as_str() == "v"));

        let int_kv = OtelKeyValue {
            key: sv("i"),
            value_type: OtelAttributeType::Int64 as u32,
            value: OtelAttributeValue { int64_value: -42 },
        };
        assert!(matches!(
            unsafe { int_kv.to_key_value() }.unwrap().value,
            Value::I64(-42)
        ));

        let bool_kv = OtelKeyValue {
            key: sv("b"),
            value_type: OtelAttributeType::Bool as u32,
            value: OtelAttributeValue { bool_value: 1 },
        };
        assert!(matches!(
            unsafe { bool_kv.to_key_value() }.unwrap().value,
            Value::Bool(true)
        ));

        let double_kv = OtelKeyValue {
            key: sv("d"),
            value_type: OtelAttributeType::Double as u32,
            value: OtelAttributeValue { double_value: 1.5 },
        };
        assert!(
            matches!(unsafe { double_kv.to_key_value() }.unwrap().value, Value::F64(v) if v == 1.5)
        );
    }

    #[test]
    fn empty_key_is_rejected() {
        let kv = OtelKeyValue {
            key: sv(""),
            value_type: OtelAttributeType::Int64 as u32,
            value: OtelAttributeValue { int64_value: 1 },
        };
        assert_eq!(
            unsafe { kv.to_key_value() }.unwrap_err(),
            OtelStatus::InvalidArgument
        );
    }

    #[test]
    fn out_of_range_value_type_is_rejected_not_ub() {
        // A bogus tag must be rejected without ever reading the union.
        let kv = OtelKeyValue {
            key: sv("k"),
            value_type: 9999,
            value: OtelAttributeValue { int64_value: 0 },
        };
        assert_eq!(
            unsafe { kv.to_key_value() }.unwrap_err(),
            OtelStatus::InvalidArgument
        );
    }

    #[test]
    fn attribute_type_from_u32_validates() {
        assert_eq!(
            OtelAttributeType::from_u32(0),
            Some(OtelAttributeType::String)
        );
        assert_eq!(
            OtelAttributeType::from_u32(3),
            Some(OtelAttributeType::Double)
        );
        assert_eq!(OtelAttributeType::from_u32(4), None);
        assert_eq!(OtelAttributeType::from_u32(u32::MAX), None);
    }

    #[test]
    fn collect_handles_empty_and_null() {
        assert!(unsafe { collect_key_values(std::ptr::null(), 0) }
            .unwrap()
            .is_empty());
        assert_eq!(
            unsafe { collect_key_values(std::ptr::null(), 2) }.unwrap_err(),
            OtelStatus::InvalidArgument
        );
    }

    #[test]
    fn collect_reads_array() {
        let kvs = [
            OtelKeyValue {
                key: sv("a"),
                value_type: OtelAttributeType::Int64 as u32,
                value: OtelAttributeValue { int64_value: 1 },
            },
            OtelKeyValue {
                key: sv("b"),
                value_type: OtelAttributeType::Int64 as u32,
                value: OtelAttributeValue { int64_value: 2 },
            },
        ];
        let out = unsafe { collect_key_values(kvs.as_ptr(), kvs.len()) }.unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].key.as_str(), "a");
        assert_eq!(out[1].key.as_str(), "b");
    }

    #[test]
    fn huge_count_overflowing_isize_is_rejected() {
        // A count whose total byte length overflows must be rejected *before* the array
        // is dereferenced. A non-NULL dangling pointer proves no read occurs: the size
        // check precedes `slice::from_raw_parts`.
        let dangling =
            std::ptr::NonNull::<OtelKeyValue>::dangling().as_ptr() as *const OtelKeyValue;
        assert_eq!(
            unsafe { collect_key_values(dangling, usize::MAX) }.unwrap_err(),
            OtelStatus::InvalidArgument
        );
    }

    #[test]
    fn oversized_count_fails_reservation_gracefully() {
        // A count that passes the `isize::MAX` byte-length check but is far too large to
        // allocate must fail via `try_reserve` (returning an error), never aborting.
        // Reservation happens before the slice is formed, so the dangling pointer is
        // never read and no huge allocation actually occurs.
        let dangling =
            std::ptr::NonNull::<OtelKeyValue>::dangling().as_ptr() as *const OtelKeyValue;
        let count = (isize::MAX as usize) / std::mem::size_of::<OtelKeyValue>();
        assert_eq!(
            unsafe { collect_key_values(dangling, count) }.unwrap_err(),
            OtelStatus::InternalError
        );
    }

    #[test]
    fn bool_tag_reads_nonzero_as_true() {
        let false_kv = OtelKeyValue {
            key: sv("b"),
            value_type: OtelAttributeType::Bool as u32,
            value: OtelAttributeValue { bool_value: 0 },
        };
        assert!(matches!(
            unsafe { false_kv.to_key_value() }.unwrap().value,
            Value::Bool(false)
        ));
        let true_kv = OtelKeyValue {
            key: sv("b"),
            value_type: OtelAttributeType::Bool as u32,
            value: OtelAttributeValue { bool_value: 42 },
        };
        assert!(matches!(
            unsafe { true_kv.to_key_value() }.unwrap().value,
            Value::Bool(true)
        ));
    }
}
