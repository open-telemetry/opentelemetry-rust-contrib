//! Zero-allocation hex encoders for `TraceId` and `SpanId`.
//!
//! `TraceId::to_string()` / `SpanId::to_string()` each allocate a heap
//! `String` on every call. On the export hot path that's two small heap
//! allocations per record with a trace context, just to format already-known
//! fixed-size data. These helpers write the hex representation directly into
//! a stack-allocated fixed-size byte array instead.

use opentelemetry::trace::{SpanId, TraceId};

const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

fn encode<const N: usize>(bytes: &[u8]) -> [u8; N] {
    debug_assert_eq!(N, bytes.len() * 2);
    let mut out = [0u8; N];
    for (i, &b) in bytes.iter().enumerate() {
        out[i * 2] = HEX_CHARS[(b >> 4) as usize];
        out[i * 2 + 1] = HEX_CHARS[(b & 0x0f) as usize];
    }
    out
}

/// Encodes a `TraceId` as 32 lowercase hex chars in a stack-allocated array.
pub(crate) fn trace_id_hex(id: TraceId) -> [u8; 32] {
    encode(&id.to_bytes())
}

/// Encodes a `SpanId` as 16 lowercase hex chars in a stack-allocated array.
pub(crate) fn span_id_hex(id: SpanId) -> [u8; 16] {
    encode(&id.to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_id_matches_to_string() {
        let id = TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e4736").unwrap();
        assert_eq!(
            std::str::from_utf8(&trace_id_hex(id)).unwrap(),
            id.to_string()
        );
    }

    #[test]
    fn span_id_matches_to_string() {
        let id = SpanId::from_hex("00f067aa0ba902b7").unwrap();
        assert_eq!(
            std::str::from_utf8(&span_id_hex(id)).unwrap(),
            id.to_string()
        );
    }

    #[test]
    fn trace_id_all_zeros() {
        let id = TraceId::from_hex("00000000000000000000000000000000").unwrap();
        assert_eq!(&trace_id_hex(id), b"00000000000000000000000000000000");
    }

    #[test]
    fn trace_id_all_ff_preserves_leading_chars() {
        let id = TraceId::from_hex("ffffffffffffffffffffffffffffffff").unwrap();
        assert_eq!(&trace_id_hex(id), b"ffffffffffffffffffffffffffffffff");
    }

    #[test]
    fn span_id_with_leading_zero_byte_is_zero_padded() {
        // Regression: ensure leading zero bytes do not get truncated
        // (the original to_string() bug fixed by user-events-logs #612).
        let id = SpanId::from_hex("0001020304050607").unwrap();
        assert_eq!(&span_id_hex(id), b"0001020304050607");
    }
}
