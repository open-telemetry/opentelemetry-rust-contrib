//! Minimal UUIDv4 generation.
//!
//! In order to avoid third-party dependencies, we generate UUIDv4s using only
//! the standard library. The result will be unique, but not cryptographically
//! secure, which is sufficient for service instance IDs. Randomness comes
//! from an OS-seeded [`RandomState`], with time, process id, and a counter
//! mixed in so repeated calls are extremely unlikely to collide.
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a random UUIDv4 as a canonical hyphenated string,
///  e.g. `"f47ac10b-58cc-4372-a567-0e02b2c3d479"`.
pub(crate) fn v4() -> String {
    let mut bytes = random_bytes();

    // https://www.rfc-editor.org/rfc/rfc9562.html#name-version-field
    // Set the high nibble of bytes[6] to 0100. This is the version.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;

    // https://www.rfc-editor.org/rfc/rfc9562.html#name-variant-field
    // Set the two most significant bits of bytes[8] to 10. This is the variant.
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    format(&bytes)
}

/// Produce 128 random bits from two calls to `seed_word()`.
fn random_bytes() -> [u8; 16] {
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&seed_word().to_ne_bytes());
    out[8..].copy_from_slice(&seed_word().to_ne_bytes());
    out
}

/// Produce 64 random bits. Each call uses an independently seeded RandomState.
/// Time, process ID, and a counter are mixed in to ensure successive calls
/// produce different output.
fn seed_word() -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    hasher.write_u64(nanos);
    hasher.write_u32(std::process::id());
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    hasher.finish()
}

/// Format the bytes as a canonical hyphenated UUIDv4 string.
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
    use super::v4;

    #[test]
    fn formats_canonical_v4() {
        let uuid = v4();

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

        // Version nibble is 4.
        assert_eq!(bytes[14], b'4');
        // Variant nibble is one of 8, 9, a, b.
        assert!(matches!(bytes[19], b'8' | b'9' | b'a' | b'b'));
    }

    #[test]
    fn generates_distinct_values() {
        let count = 1000;
        let unique: std::collections::HashSet<_> = (0..count).map(|_| v4()).collect();
        assert_eq!(unique.len(), count);
    }
}
