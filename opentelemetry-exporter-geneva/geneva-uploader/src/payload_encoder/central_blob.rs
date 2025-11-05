//use md5;

use crate::payload_encoder::bond_encoder::BondEncodedSchema;
use chrono::{DateTime, Datelike, Timelike, Utc};
use std::sync::Arc;

/// Metadata for a batch of events
#[derive(Debug, Clone)]
pub struct BatchMetadata {
    /// Start time of the earliest event in nanoseconds since Unix epoch
    pub start_time: u64,
    /// End time of the latest event in nanoseconds since Unix epoch
    pub end_time: u64,
    /// Schema IDs present in this batch formatted as MD5 hashes separated by semicolons
    pub schema_ids: String,
}

impl BatchMetadata {
    /// Format start timestamp in ISO 8601 format with 7-digit precision (.NET compatible)
    #[inline]
    pub(crate) fn format_start_timestamp(&self) -> String {
        Self::format_timestamp(self.start_time)
    }

    /// Format end timestamp in ISO 8601 format with 7-digit precision (.NET compatible)
    #[inline]
    pub(crate) fn format_end_timestamp(&self) -> String {
        Self::format_timestamp(self.end_time)
    }

    /// Format timestamp using DateTime::from_timestamp_nanos
    #[inline]
    fn format_timestamp(timestamp_nanos: u64) -> String {
        let datetime = DateTime::from_timestamp_nanos(timestamp_nanos as i64);
        Self::format_datetime(datetime)
    }

    /// Format datetime as ISO 8601 with 7-digit precision
    /// TODO: Revisit using datetime.format("%Y-%m-%dT%H:%M:%S%.7fZ") once chrono Display implementation is fixed
    #[inline]
    fn format_datetime(datetime: DateTime<Utc>) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:07}Z",
            datetime.year(),
            datetime.month(),
            datetime.day(),
            datetime.hour(),
            datetime.minute(),
            datetime.second(),
            datetime.nanosecond() / 100 // Convert nanoseconds to 7-digit precision
        )
    }
}

/// Helper to encode UTF-8 Rust str to UTF-16LE bytes
/// TODO - consider avoiding temporary allocation, by passing a mutable buffer
#[allow(dead_code)]
fn utf8_to_utf16le_bytes(s: &str) -> Vec<u8> {
    // Each UTF-16 code unit is 2 bytes. For ASCII strings, the UTF-16 representation
    // will be twice as large as UTF-8. For non-ASCII strings, UTF-16 may be more compact
    // than UTF-8 in some cases, but to avoid reallocations we preallocate 2x len.
    let mut buf = Vec::with_capacity(s.len() * 2);
    for u in s.encode_utf16() {
        buf.extend_from_slice(&u.to_le_bytes());
    }
    buf
}

/// Schema entry for central blob
#[allow(dead_code)]
pub(crate) struct CentralSchemaEntry {
    pub id: u64,
    pub md5: [u8; 16],
    pub schema: BondEncodedSchema,
}

/// Event/row entry for central blob
#[allow(dead_code)]
pub(crate) struct CentralEventEntry {
    pub schema_id: u64,
    pub level: u8,
    pub event_name: Arc<String>,
    pub row: Vec<u8>,
}

const TERMINATOR: u64 = 0xdeadc0dedeadc0de;

/// CentralBlob Protocol Payload Structure
///
/// The payload consists of a header, metadata, schemas, and events, each encoded in a specific format.
/// The central terminator constant used throughout is `TERMINATOR = 0xdeadc0dedeadc0de`.
///
/// ## Payload Structure
///
/// ### Header
/// - **Version**: `u32` (4 bytes)
/// - **Format**: `u32` (4 bytes)
///
/// ### Metadata
/// - **Length**: `u32` (4 bytes, prefix for UTF-16LE encoded metadata)
/// - **Metadata**: UTF-16LE encoded string (variable length)
///
/// ### Schemas
/// A collection of schema entries, each encoded as follows:
/// - **Entity Type**: `u16` (2 bytes, value = 0)
/// - **Schema ID**: `u64` (8 bytes, unique identifier)
/// - **MD5 Hash**: `[u8; 16]` (16 bytes, MD5 checksum of schema bytes)
/// - **Schema Length**: `u32` (4 bytes)
/// - **Schema Bytes**: `Vec<u8>` (variable length, schema serialized as bytes)
/// - **Terminator**: `u64` (8 bytes, constant `TERMINATOR`)
///
/// ### Events
/// A collection of event entries, each encoded as follows:
/// - **Entity Type**: `u16` (2 bytes, value = 2)
/// - **Schema ID**: `u64` (8 bytes, links the event to a schema)
/// - **Level**: `u8` (1 byte, event verbosity or severity level)
/// - **Event Name Length**: `u16` (2 bytes, prefix for UTF-16LE encoded event name)
/// - **Event Name**: UTF-16LE encoded string (variable length)
/// - **Row Length**: `u32` (4 bytes, prefix for row data including `Simple Protocol` header)
/// - **Row Data**:
///   - **Simple Protocol Header**: `[0x53, 0x50, 0x01, 0x00]` (4 bytes)
///   - **Row Bytes**: `Vec<u8>` (variable length, row serialized as bytes)
/// - **Terminator**: `u64` (8 bytes, constant `TERMINATOR`)
///
#[allow(dead_code)]
pub(crate) struct CentralBlob {
    pub version: u32,
    pub format: u32,
    pub metadata: String, // UTF-8, will be stored as UTF-16LE
    pub schemas: Vec<CentralSchemaEntry>,
    pub events: Vec<CentralEventEntry>,
}

impl CentralBlob {
    #[allow(dead_code)]
    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        // Estimate buffer size:
        // - Header: 4 (version) + 4 (format)
        // - Metadata: 4 (length prefix) + metadata_utf16.len()
        // - Each schema:
        //     2 (entity type, u16)
        //   + 8 (schema id, u64)
        //   + 16 (md5, [u8;16])
        //   + 4 (schema bytes length, u32)
        //   + schema_bytes.len()
        //   + 8 (terminator, u64)
        // - Each event:
        //     2 (entity type, u16)
        //   + 8 (schema_id, u64)
        //   + 1 (level, u8)
        //   + 2 (event name length, u16)
        //   + event_name_utf16.len()
        //   + 4 (row length, u32)
        //   + 4 (Simple Protocol header)
        //   + row_bytes.len()
        //   + 8 (terminator, u64)
        let meta_utf16 = utf8_to_utf16le_bytes(&self.metadata);
        let events_with_utf16 = self
            .events
            .iter()
            .map(|e| {
                let evname_utf16 = utf8_to_utf16le_bytes(&e.event_name);
                (e, evname_utf16)
            })
            .collect::<Vec<_>>();
        let mut estimated_size = 8 + 4 + meta_utf16.len();
        estimated_size += self
            .schemas
            .iter()
            .map(|s| 2 + 8 + 16 + 4 + s.schema.as_bytes().len() + 8)
            .sum::<usize>();
        estimated_size += events_with_utf16
            .iter()
            .map(|(e, evname_utf16)| {
                let row_len = {
                    4 + &e.row.len() // SP header (4), row_bytes
                };
                2 + 8 + 1 + 2 + 4 + evname_utf16.len() + row_len + 8
            })
            .sum::<usize>();

        let mut buf = Vec::with_capacity(estimated_size);

        // HEADER
        buf.extend_from_slice(&self.version.to_le_bytes());
        buf.extend_from_slice(&self.format.to_le_bytes());

        // METADATA (len, UTF-16LE bytes)
        buf.extend_from_slice(&(meta_utf16.len() as u32).to_le_bytes());
        buf.extend_from_slice(&meta_utf16);

        // SCHEMAS (type 0)
        for schema in &self.schemas {
            buf.extend_from_slice(&0u16.to_le_bytes()); // entity type 0
            buf.extend_from_slice(&schema.id.to_le_bytes());
            buf.extend_from_slice(&schema.md5);
            let schema_bytes = schema.schema.as_bytes();
            buf.extend_from_slice(&(schema_bytes.len() as u32).to_le_bytes()); //TODO - check for overflow
            buf.extend_from_slice(schema_bytes);
            buf.extend_from_slice(&TERMINATOR.to_le_bytes());
        }

        // EVENTS (type 2)
        for (event, evname_utf16) in events_with_utf16 {
            buf.extend_from_slice(&2u16.to_le_bytes()); // entity type 2
            buf.extend_from_slice(&event.schema_id.to_le_bytes());
            buf.push(event.level);

            // event name (UTF-16LE, prefixed with u16 len in bytes)
            buf.extend_from_slice(&(evname_utf16.len() as u16).to_le_bytes()); // TODO - check for overflow
            buf.extend_from_slice(&evname_utf16);

            let total_len = 4 + &event.row.len(); // SP header + data

            buf.extend_from_slice(&(total_len as u32).to_le_bytes()); // TODO - check for overflow
            buf.extend_from_slice(&[0x53, 0x50, 0x01, 0x00]); // Simple Protocol header
            buf.extend_from_slice(&event.row);

            buf.extend_from_slice(&TERMINATOR.to_le_bytes());
        }

        buf
    }
}

// Example usage/test (can be moved to examples or tests)
#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload_encoder::bond_encoder::{BondEncodedSchema, FieldDef};
    use std::borrow::Cow;

    //Helper to calculate MD5 hash, returns [u8;16]
    fn md5_bytes(data: &[u8]) -> [u8; 16] {
        md5::compute(data).0
    }

    #[test]
    fn test_central_blob_creation() {
        // Prepare a schema
        let fields = vec![
            FieldDef {
                name: Cow::Borrowed("foo"),
                type_id: crate::payload_encoder::bond_encoder::BondDataType::BT_INT32,
                field_id: 1u16,
            },
            FieldDef {
                name: Cow::Borrowed("bar"),
                type_id: crate::payload_encoder::bond_encoder::BondDataType::BT_STRING,
                field_id: 2u16,
            },
        ];
        let schema_obj = BondEncodedSchema::from_fields("TestStruct", "test.namespace", fields);
        let schema_bytes = schema_obj.as_bytes().to_vec();
        let schema_md5 = md5_bytes(&schema_bytes);
        let schema_id = 1234u64;

        let schema = CentralSchemaEntry {
            id: schema_id,
            md5: schema_md5,
            schema: schema_obj,
        };

        // Prepare a row
        let mut row = Vec::new();
        row.extend_from_slice(&42i32.to_le_bytes());
        let s = "hello";
        row.extend_from_slice(&(s.len() as u32).to_le_bytes());
        row.extend_from_slice(s.as_bytes());

        let event = CentralEventEntry {
            schema_id,
            level: 0, // e.g. ETW verbose
            event_name: Arc::new("eventname".to_string()),
            row,
        };

        // Metadata
        let metadata =
            "namespace=testNamespace/eventVersion=Ver1v0/tenant=T/role=R/roleinstance=RI";

        // Build blob
        let blob = CentralBlob {
            version: 1,
            format: 42,
            metadata: metadata.to_string(),
            schemas: vec![schema],
            events: vec![event],
        };

        let payload = blob.to_bytes();

        // Only assert that the payload is created and non-empty
        assert!(!payload.is_empty());
    }
}
