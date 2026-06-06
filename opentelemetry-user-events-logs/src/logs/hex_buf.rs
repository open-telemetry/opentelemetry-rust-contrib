//! Zero-allocation hex string encoder for fixed-size byte arrays such as
//! `TraceId` (16 bytes -> 32 hex chars) and `SpanId` (8 bytes -> 16 hex chars).
//!
//! `TraceId::to_string()` / `SpanId::to_string()` each allocate a heap
//! `String` on every call. On the export hot path that's two small heap
//! allocations per record with a trace context, just to format already-known
//! fixed-size data. This helper writes the hex representation directly into
//! a stack-allocated `[u8; N]` buffer instead.

const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

/// Stack-allocated lowercase-hex buffer of fixed byte length `N`.
///
/// `N` must equal `2 * input_bytes.len()` (enforced by `debug_assert!`).
pub(crate) struct HexBuf<const N: usize>([u8; N]);

impl<const N: usize> HexBuf<N> {
    /// Encodes `bytes` as lowercase hex into a stack buffer of length `N`.
    pub(crate) fn from_bytes(bytes: &[u8]) -> Self {
        debug_assert_eq!(N, bytes.len() * 2, "HexBuf<N> requires N == 2 * bytes.len()");
        let mut out = [0u8; N];
        for (i, &b) in bytes.iter().enumerate() {
            out[i * 2] = HEX_CHARS[(b >> 4) as usize];
            out[i * 2 + 1] = HEX_CHARS[(b & 0x0f) as usize];
        }
        Self(out)
    }

    /// Returns the hex representation as a byte slice.
    ///
    /// `eventheader_dynamic::EventBuilder::add_str` accepts any
    /// `AsRef<[V: StringField]>`, and `u8: StringField`, so callers can pass
    /// these bytes directly without UTF-8 validation or an `unsafe` block.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::{SpanId, TraceId};

    #[test]
    fn trace_id_matches_to_string() {
        let trace_id = TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e4736").unwrap();
        let buf = HexBuf::<32>::from_bytes(&trace_id.to_bytes());
        assert_eq!(std::str::from_utf8(buf.as_bytes()).unwrap(), trace_id.to_string());
    }

    #[test]
    fn span_id_matches_to_string() {
        let span_id = SpanId::from_hex("00f067aa0ba902b7").unwrap();
        let buf = HexBuf::<16>::from_bytes(&span_id.to_bytes());
        assert_eq!(std::str::from_utf8(buf.as_bytes()).unwrap(), span_id.to_string());
    }

    #[test]
    fn trace_id_all_zeros() {
        let trace_id = TraceId::from_hex("00000000000000000000000000000000").unwrap();
        let buf = HexBuf::<32>::from_bytes(&trace_id.to_bytes());
        assert_eq!(std::str::from_utf8(buf.as_bytes()).unwrap(), "00000000000000000000000000000000");
    }

    #[test]
    fn trace_id_all_ff_preserves_leading_chars() {
        let trace_id = TraceId::from_hex("ffffffffffffffffffffffffffffffff").unwrap();
        let buf = HexBuf::<32>::from_bytes(&trace_id.to_bytes());
        assert_eq!(std::str::from_utf8(buf.as_bytes()).unwrap(), "ffffffffffffffffffffffffffffffff");
    }

    #[test]
    fn span_id_with_leading_zero_byte_is_zero_padded() {
        // Regression: ensure leading zero bytes do not get truncated
        // (the original to_string() bug fixed by user-events-logs #612).
        let span_id = SpanId::from_hex("0001020304050607").unwrap();
        let buf = HexBuf::<16>::from_bytes(&span_id.to_bytes());
        assert_eq!(std::str::from_utf8(buf.as_bytes()).unwrap(), "0001020304050607");
        assert_eq!(std::str::from_utf8(buf.as_bytes()).unwrap().len(), 16);
    }
}
