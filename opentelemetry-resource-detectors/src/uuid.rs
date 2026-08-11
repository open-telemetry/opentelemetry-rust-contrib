//! Minimal UUIDv7 generation.
//!
//! In order to avoid third-party dependencies, we generate UUIDv7s using only
//! the standard library. The result will be unique, but not cryptographically
//! secure, which is sufficient for service instance IDs. Randomness comes
//! from an OS-seeded [`RandomState`], with process id and a counter mixed in
//! so repeated calls are extremely unlikely to collide.
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a UUIDv7 as a canonical hyphenated string,
/// e.g. `"017f22e2-79b0-7cc3-98c4-dc0c0c07398f"`.
pub(crate) fn v7() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let rand_hi = seed_word();
    let rand_lo = seed_word();

    let mut bytes = [0u8; 16];

    // https://www.rfc-editor.org/rfc/rfc9562.html#name-uuid-version-7
    // Bytes 0-5: 48-bit Unix timestamp in milliseconds, big-endian.
    bytes[0] = (ms >> 40) as u8;
    bytes[1] = (ms >> 32) as u8;
    bytes[2] = (ms >> 24) as u8;
    bytes[3] = (ms >> 16) as u8;
    bytes[4] = (ms >> 8) as u8;
    bytes[5] = ms as u8;

    // Bytes 6-15: random data.
    bytes[6..14].copy_from_slice(&rand_hi.to_ne_bytes());
    bytes[14..16].copy_from_slice(&rand_lo.to_ne_bytes()[..2]);

    // https://www.rfc-editor.org/rfc/rfc9562.html#name-version-field
    // Set the high nibble of bytes[6] to 0111. This is the version.
    bytes[6] = (bytes[6] & 0x0f) | 0x70;

    // https://www.rfc-editor.org/rfc/rfc9562.html#name-variant-field
    // Set the two most significant bits of bytes[8] to 10. This is the variant.
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    format(&bytes)
}

/// Produce 64 random bits. Each call uses an independently seeded RandomState.
/// Process ID and a counter are mixed in to ensure successive calls produce
/// different output.
fn seed_word() -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    hasher.finish()
}

/// Format the bytes as a canonical hyphenated UUID string.
fn format(bytes: &[u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(36);
    for (i, byte) in bytes.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            s.push('-');
        }
        // Push the high nibble then the low nibble as hex chars.
        s.push(HEX[(byte >> 4) as usize] as char);
        s.push(HEX[(byte & 0x0f) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::v7;

    #[test]
    fn formats_canonical_v7() {
        let uuid = v7();

        // 32 hex digits + 4 hyphens.
        assert_eq!(uuid.len(), 36);

        let bytes = uuid.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            if matches!(i, 8 | 13 | 18 | 23) {
                assert_eq!(b, b'-', "expected hyphen at index {i}");
            } else {
                assert!(b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
            }
        }

        // Version nibble is 7.
        assert_eq!(bytes[14], b'7');
        // Variant nibble is one of 8, 9, a, b.
        assert!(matches!(bytes[19], b'8' | b'9' | b'a' | b'b'));
    }

    #[test]
    fn generates_distinct_values() {
        let count = 1000;
        let unique: std::collections::HashSet<_> = (0..count).map(|_| v7()).collect();
        assert_eq!(unique.len(), count);
    }

    #[test]
    fn timestamp_is_recent() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let uuid = v7();
        // Extract the 48-bit timestamp from the first 12 hex chars
        let hex: String = uuid.chars().filter(|c| *c != '-').take(12).collect();

        // Convert the hex string to a u64 timestamp in milliseconds.
        let ts_ms = u64::from_str_radix(&hex, 16).expect("valid hex");

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // Timestamp should be within 5 seconds of now.
        assert!(ts_ms <= now_ms, "timestamp is in the future");
        assert!(now_ms - ts_ms < 5_000, "timestamp is too old");
    }
}
