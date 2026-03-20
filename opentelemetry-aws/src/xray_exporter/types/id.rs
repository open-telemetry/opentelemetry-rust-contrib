use super::error::ConstraintError;
use core::{fmt, str::FromStr};
use opentelemetry::{SpanId, TraceId as OtelTraceId};
use serde::Serialize;
// use std::time::{SystemTime, UNIX_EPOCH};

/// 64-bit identifier for X-Ray segments and subsegments.
///
/// Serialized as 16 hexadecimal characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(u64);

impl Id {
    /// Creates a new random segment ID.
    pub fn new() -> Self {
        Self(rand::random())
    }
}

impl Default for Id {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl FromStr for Id {
    type Err = ConstraintError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 16 {
            return Err(ConstraintError::InvalidId);
        }

        let id = u64::from_str_radix(s, 16).map_err(|_| ConstraintError::InvalidId)?;
        Ok(Self(id))
    }
}

impl From<u64> for Id {
    fn from(id: u64) -> Self {
        Self(id)
    }
}
impl From<SpanId> for Id {
    fn from(value: SpanId) -> Self {
        Self::from(u64::from_be_bytes(value.to_bytes()))
    }
}

impl Serialize for Id {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(&self)
    }
}

/// 128-bit identifier for X-Ray traces.
///
/// Formatted as `1-{timestamp}-{random}` where timestamp is 8 hex digits (seconds since epoch)
/// and random is 24 hex digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TraceId {
    timestamp: u32,
    id_msb: u32,
    id_lsb: u64,
}

impl Default for TraceId {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceId {
    /// Creates a new trace ID with the current timestamp.
    pub fn new() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as u32;
        Self {
            timestamp,
            id_msb: rand::random(),
            id_lsb: rand::random(),
        }
    }

    /// Returns the timestamp portion in seconds since Unix epoch.
    pub(crate) fn timestamp(&self) -> u32 {
        self.timestamp
    }
}

impl fmt::Display for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Format as X-Ray trace ID: 1-{8 hex}-{24 hex}
        write!(
            f,
            "1-{:08x}-{:08x}{:016x}",
            self.timestamp, self.id_msb, self.id_lsb,
        )
    }
}

impl FromStr for TraceId {
    type Err = ConstraintError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 35 {
            return Err(ConstraintError::InvalidTraceId);
        }

        let mut parts = s.split('-');
        let Some("1") = parts.next() else {
            return Err(ConstraintError::InvalidTraceId);
        };

        let timestamp = parts
            .next()
            .ok_or(ConstraintError::InvalidTraceId)
            .and_then(|s| {
                u32::from_str_radix(s, 16).map_err(|_| ConstraintError::InvalidTraceId)
            })?;

        let (id_msb, id_lsb) = parts
            .next()
            .ok_or(ConstraintError::InvalidTraceId)
            .and_then(|s| {
                let (msb, lsb) = s.split_at(8);
                Ok((
                    u32::from_str_radix(msb, 16).map_err(|_| ConstraintError::InvalidTraceId)?,
                    u64::from_str_radix(lsb, 16).map_err(|_| ConstraintError::InvalidTraceId)?,
                ))
            })?;

        Ok(Self {
            timestamp,
            id_msb,
            id_lsb,
        })
    }
}

impl From<u128> for TraceId {
    fn from(id: u128) -> Self {
        Self {
            timestamp: (id >> 96) as u32,
            id_msb: (id >> 64) as u32,
            id_lsb: id as u64,
        }
    }
}

impl From<OtelTraceId> for TraceId {
    fn from(value: OtelTraceId) -> Self {
        Self::from(u128::from_be_bytes(value.to_bytes()))
    }
}

impl Serialize for TraceId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(&self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    // Helper function to create Id from u64
    fn create_id(value: u64) -> Id {
        Id(value)
    }

    // Helper function to create TraceId from components
    fn create_trace_id(timestamp: u32, id_msb: u32, id_lsb: u64) -> TraceId {
        TraceId {
            timestamp,
            id_msb,
            id_lsb,
        }
    }

    // Tests for Id::from_str

    #[test]
    fn id_from_str_valid() {
        // Standard hex string
        let id = Id::from_str("1234567890abcdef").unwrap();
        assert_eq!(format!("{id}"), "1234567890abcdef");

        // All zeros
        let id = Id::from_str("0000000000000000").unwrap();
        assert_eq!(id, create_id(0));

        // All f's (max value)
        let id = Id::from_str("ffffffffffffffff").unwrap();
        assert_eq!(id, create_id(u64::MAX));

        // Uppercase hex should work
        let id = Id::from_str("ABCDEF0123456789").unwrap();
        assert_eq!(format!("{id}"), "abcdef0123456789");
    }

    #[test]
    fn id_from_str_invalid() {
        // Too short
        assert!(matches!(
            Id::from_str("123456789abcdef"),
            Err(ConstraintError::InvalidId)
        ));

        // Too long
        assert!(matches!(
            Id::from_str("1234567890abcdef0"),
            Err(ConstraintError::InvalidId)
        ));

        // Non-hex characters
        assert!(matches!(
            Id::from_str("123456789abcdefg"),
            Err(ConstraintError::InvalidId)
        ));

        // Empty string
        assert!(matches!(Id::from_str(""), Err(ConstraintError::InvalidId)));

        // Special characters
        assert!(matches!(
            Id::from_str("1234567890abcd-f"),
            Err(ConstraintError::InvalidId)
        ));
    }

    // Tests for Id::from<SpanId>

    #[test]
    fn id_from_span_id() {
        // Create SpanId from bytes (big-endian)
        let span_id = SpanId::from_bytes([0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef]);
        let id = Id::from(span_id);
        assert_eq!(format!("{id}"), "1234567890abcdef");

        // Zero SpanId
        let span_id = SpanId::from_bytes([0; 8]);
        let id = Id::from(span_id);
        assert_eq!(id, create_id(0));

        // Max SpanId
        let span_id = SpanId::from_bytes([0xff; 8]);
        let id = Id::from(span_id);
        assert_eq!(id, create_id(u64::MAX));
    }

    // Tests for Id::serialize

    #[test]
    fn id_serialize() {
        // Standard value
        let id = create_id(0x1234567890abcdef);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"1234567890abcdef\"");

        // Zero
        let id = create_id(0);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"0000000000000000\"");

        // Max value
        let id = create_id(u64::MAX);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"ffffffffffffffff\"");
    }

    // Tests for TraceId::fmt (Display)

    #[test]
    fn trace_id_display_valid() {
        // Standard trace ID
        let trace_id = create_trace_id(0x5f5e100a, 0x12345678, 0x90abcdef01234567);
        let formatted = format!("{trace_id}");
        assert_eq!(formatted, "1-5f5e100a-1234567890abcdef01234567");

        // Verify format structure: 1-{8hex}-{24hex}
        assert!(formatted.starts_with("1-"));
        let parts: Vec<&str> = formatted.split('-').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "1");
        assert_eq!(parts[1].len(), 8);
        assert_eq!(parts[2].len(), 24);

        // Another example with different values
        let trace_id = create_trace_id(0xabcdef01, 0xfedcba98, 0x7654321001234567);
        assert_eq!(format!("{trace_id}"), "1-abcdef01-fedcba987654321001234567");

        // All zeros
        let trace_id = create_trace_id(0, 0, 0);
        let formatted = format!("{trace_id}");
        assert_eq!(formatted, "1-00000000-000000000000000000000000");
        assert_eq!(formatted.len(), 35);

        // Max values
        let trace_id = create_trace_id(u32::MAX, u32::MAX, u64::MAX);
        let formatted = format!("{trace_id}");
        assert_eq!(formatted, "1-ffffffff-ffffffffffffffffffffffff");
        assert_eq!(formatted.len(), 35);

        // Mixed values with leading zeros
        let trace_id = create_trace_id(1, 2, 3);
        let formatted = format!("{trace_id}");
        assert_eq!(formatted, "1-00000001-000000020000000000000003");
    }

    // Tests for TraceId::from_str

    #[test]
    fn trace_id_from_str_valid() {
        // Standard X-Ray trace ID
        let trace_id = TraceId::from_str("1-5f5e100a-1234567890abcdef01234567").unwrap();
        assert_eq!(trace_id.timestamp, 0x5f5e100a);
        assert_eq!(format!("{trace_id}"), "1-5f5e100a-1234567890abcdef01234567");

        // All zeros
        let trace_id = TraceId::from_str("1-00000000-000000000000000000000000").unwrap();
        assert_eq!(trace_id, create_trace_id(0, 0, 0));

        // All f's
        let trace_id = TraceId::from_str("1-ffffffff-ffffffffffffffffffffffff").unwrap();
        assert_eq!(trace_id, create_trace_id(u32::MAX, u32::MAX, u64::MAX));

        // Uppercase hex should work
        let trace_id = TraceId::from_str("1-ABCDEF01-FEDCBA987654321001234567").unwrap();
        assert_eq!(trace_id.timestamp, 0xabcdef01);
    }

    #[test]
    fn trace_id_from_str_invalid() {
        // Wrong version number
        assert!(matches!(
            TraceId::from_str("2-5f5e100a-1234567890abcdef01234567"),
            Err(ConstraintError::InvalidTraceId)
        ));

        // Too short
        assert!(matches!(
            TraceId::from_str("1-5f5e100a-1234567890abcdef0123456"),
            Err(ConstraintError::InvalidTraceId)
        ));

        // Too long
        assert!(matches!(
            TraceId::from_str("1-5f5e100a-1234567890abcdef012345678"),
            Err(ConstraintError::InvalidTraceId)
        ));

        // Missing parts
        assert!(matches!(
            TraceId::from_str("1-5f5e100a"),
            Err(ConstraintError::InvalidTraceId)
        ));

        // Wrong timestamp length (7 instead of 8)
        assert!(matches!(
            TraceId::from_str("1-5f5e10a-1234567890abcdef01234567"),
            Err(ConstraintError::InvalidTraceId)
        ));

        // Non-hex characters in timestamp
        assert!(matches!(
            TraceId::from_str("1-5f5e10ag-1234567890abcdef01234567"),
            Err(ConstraintError::InvalidTraceId)
        ));

        // Non-hex characters in random part
        assert!(matches!(
            TraceId::from_str("1-5f5e100a-1234567890abcdefg1234567"),
            Err(ConstraintError::InvalidTraceId)
        ));

        // Empty string
        assert!(matches!(
            TraceId::from_str(""),
            Err(ConstraintError::InvalidTraceId)
        ));

        // Wrong separator
        assert!(matches!(
            TraceId::from_str("1:5f5e100a:1234567890abcdef01234567"),
            Err(ConstraintError::InvalidTraceId)
        ));
    }

    // Tests for TraceId::from<u128>

    #[test]
    fn trace_id_from_u128() {
        // Test bit extraction: timestamp (bits 96-127), msb (bits 64-95), lsb (bits 0-63)
        #[allow(clippy::unusual_byte_groupings)]
        let value: u128 = 0x5f5e100a_12345678_90abcdef01234567;
        let trace_id = TraceId::from(value);
        assert_eq!(trace_id.timestamp, 0x5f5e100a);
        assert_eq!(trace_id.id_msb, 0x12345678);
        assert_eq!(trace_id.id_lsb, 0x90abcdef01234567);

        // Zero
        let trace_id = TraceId::from(0u128);
        assert_eq!(trace_id, create_trace_id(0, 0, 0));

        // Max value
        let trace_id = TraceId::from(u128::MAX);
        assert_eq!(trace_id, create_trace_id(u32::MAX, u32::MAX, u64::MAX));
    }

    // Tests for TraceId::from<OtelTraceId>

    #[test]
    fn trace_id_from_otel_trace_id() {
        // Create OtelTraceId from bytes (big-endian)
        let bytes = [
            0x5f, 0x5e, 0x10, 0x0a, // timestamp
            0x12, 0x34, 0x56, 0x78, // msb
            0x90, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, // lsb
        ];
        let otel_trace_id = OtelTraceId::from_bytes(bytes);
        let trace_id = TraceId::from(otel_trace_id);

        assert_eq!(trace_id.timestamp, 0x5f5e100a);
        assert_eq!(trace_id.id_msb, 0x12345678);
        assert_eq!(trace_id.id_lsb, 0x90abcdef01234567);
        assert_eq!(format!("{trace_id}"), "1-5f5e100a-1234567890abcdef01234567");

        // Zero OtelTraceId
        let otel_trace_id = OtelTraceId::from_bytes([0; 16]);
        let trace_id = TraceId::from(otel_trace_id);
        assert_eq!(trace_id, create_trace_id(0, 0, 0));

        // Max OtelTraceId
        let otel_trace_id = OtelTraceId::from_bytes([0xff; 16]);
        let trace_id = TraceId::from(otel_trace_id);
        assert_eq!(trace_id, create_trace_id(u32::MAX, u32::MAX, u64::MAX));
    }

    // Tests for TraceId::serialize

    #[test]
    fn trace_id_serialize() {
        // Standard trace ID
        let trace_id = create_trace_id(0x5f5e100a, 0x12345678, 0x90abcdef01234567);
        let json = serde_json::to_string(&trace_id).unwrap();
        assert_eq!(json, "\"1-5f5e100a-1234567890abcdef01234567\"");

        // Zero trace ID
        let trace_id = create_trace_id(0, 0, 0);
        let json = serde_json::to_string(&trace_id).unwrap();
        assert_eq!(json, "\"1-00000000-000000000000000000000000\"");

        // Max trace ID
        let trace_id = create_trace_id(u32::MAX, u32::MAX, u64::MAX);
        let json = serde_json::to_string(&trace_id).unwrap();
        assert_eq!(json, "\"1-ffffffff-ffffffffffffffffffffffff\"");
    }
}
