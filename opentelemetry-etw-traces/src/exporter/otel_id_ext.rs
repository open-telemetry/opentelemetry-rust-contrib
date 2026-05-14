//! Zero-allocation hex string conversions for OpenTelemetry [`TraceId`] and [`SpanId`].
//!
//! Each `to_string()` call on a TraceId or SpanId
//! allocates a new `String`; these extension traits replace that with stack-allocated
//! fixed-size buffers.

use opentelemetry::trace::{SpanId, TraceId};

/// Hex lookup table for encoding bytes to lowercase hex.
const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

/// Stack-allocated hex string for a [`TraceId`] (32 hex characters).
pub(crate) struct TraceIdHex {
    buf: [u8; 32],
}

impl TraceIdHex {
    /// Returns the hex string as a byte slice.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    /// Returns the hex string as a `&str`.
    pub(crate) fn as_str(&self) -> &str {
        // SAFETY: `buf` only contains ASCII hex characters written by `hex_encode`.
        std::str::from_utf8(&self.buf).expect("hex buffer is valid UTF-8")
    }
}

/// Stack-allocated hex string for a [`SpanId`] (16 hex characters).
pub(crate) struct SpanIdHex {
    buf: [u8; 16],
}

impl SpanIdHex {
    /// Returns the hex string as a byte slice.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    /// Returns the hex string as a `&str`.
    pub(crate) fn as_str(&self) -> &str {
        // SAFETY: `buf` only contains ASCII hex characters written by `hex_encode`.
        std::str::from_utf8(&self.buf).expect("hex buffer is valid UTF-8")
    }
}

/// Hex-encodes `bytes` into `buf`. `buf` must be exactly `2 * bytes.len()`.
#[inline]
fn hex_encode(bytes: &[u8], buf: &mut [u8]) {
    debug_assert_eq!(buf.len(), bytes.len() * 2);
    for (i, &b) in bytes.iter().enumerate() {
        buf[i * 2] = HEX_CHARS[(b >> 4) as usize];
        buf[i * 2 + 1] = HEX_CHARS[(b & 0x0f) as usize];
    }
}

/// Extension trait that adds zero-allocation hex conversion to [`TraceId`].
pub(crate) trait TraceIdExt {
    /// Returns a stack-allocated hex string representation.
    fn to_hex(&self) -> TraceIdHex;
}

/// Extension trait that adds zero-allocation hex conversion to [`SpanId`].
pub(crate) trait SpanIdExt {
    /// Returns a stack-allocated hex string representation.
    fn to_hex(&self) -> SpanIdHex;
}

impl TraceIdExt for TraceId {
    fn to_hex(&self) -> TraceIdHex {
        let mut hex = TraceIdHex { buf: [0u8; 32] };
        hex_encode(&self.to_bytes(), &mut hex.buf);
        hex
    }
}

impl SpanIdExt for SpanId {
    fn to_hex(&self) -> SpanIdHex {
        let mut hex = SpanIdHex { buf: [0u8; 16] };
        hex_encode(&self.to_bytes(), &mut hex.buf);
        hex
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_id_to_hex_matches_to_string() {
        let trace_id = TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e4736").unwrap();
        let hex = trace_id.to_hex();
        assert_eq!(
            std::str::from_utf8(hex.as_bytes()).unwrap(),
            trace_id.to_string()
        );
    }

    #[test]
    fn span_id_to_hex_matches_to_string() {
        let span_id = SpanId::from_hex("00f067aa0ba902b7").unwrap();
        let hex = span_id.to_hex();
        assert_eq!(hex.as_str(), span_id.to_string());
    }

    #[test]
    fn trace_id_zero() {
        let trace_id = TraceId::from_hex("00000000000000000000000000000000").unwrap();
        let hex = trace_id.to_hex();
        assert_eq!(
            std::str::from_utf8(hex.as_bytes()).unwrap(),
            "00000000000000000000000000000000"
        );
    }

    #[test]
    fn span_id_zero() {
        let span_id = SpanId::from_hex("0000000000000000").unwrap();
        let hex = span_id.to_hex();
        assert_eq!(hex.as_str(), "0000000000000000");
    }

    #[test]
    fn trace_id_all_ff() {
        let trace_id = TraceId::from_hex("ffffffffffffffffffffffffffffffff").unwrap();
        let hex = trace_id.to_hex();
        assert_eq!(
            std::str::from_utf8(hex.as_bytes()).unwrap(),
            "ffffffffffffffffffffffffffffffff"
        );
    }

    #[test]
    fn span_id_all_ff() {
        let span_id = SpanId::from_hex("ffffffffffffffff").unwrap();
        let hex = span_id.to_hex();
        assert_eq!(hex.as_str(), "ffffffffffffffff");
    }

    #[test]
    fn hex_as_bytes_equals_as_str_bytes() {
        let span_id = SpanId::from_hex("1234567890abcdef").unwrap();
        let hex = span_id.to_hex();
        assert_eq!(hex.as_bytes(), hex.as_str().as_bytes());
    }
}
