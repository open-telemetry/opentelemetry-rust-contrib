use crate::payload_encoder::bond_encoder::{BondEncodedSchema, FieldDef};
use chrono::{DateTime, Datelike, Timelike, Utc};
use std::sync::Arc;

/// Metadata for a batch of events
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BatchMetadata {
    /// Start time of the earliest event in nanoseconds since Unix epoch
    pub(crate) start_time: u64,
    /// End time of the latest event in nanoseconds since Unix epoch
    pub(crate) end_time: u64,
    /// Schema IDs present in this batch formatted as MD5 hashes separated by semicolons
    pub(crate) schema_ids: String,
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

/// Append a string as UTF-16LE bytes without allocating a temporary buffer.
#[allow(dead_code)]
fn write_utf16le(buf: &mut Vec<u8>, s: &str) {
    for u in s.encode_utf16() {
        buf.extend_from_slice(&u.to_le_bytes());
    }
}

/// Schema entry for central blob
#[allow(dead_code)]
pub(crate) struct CentralSchemaEntry {
    pub id: u64,
    pub md5: [u8; 16],
    pub schema: BondEncodedSchema,
    // Store fields for deduplication
    // TODO: It might be faster to encode schema per event and compare encoded bytes with stored schemas
    // instead of checking each field. Needs benchmarking to determine which approach is faster.
    pub fields: Vec<FieldDef>,
}

/// Event/row entry for central blob
#[allow(dead_code)]
pub(crate) struct CentralEventEntry {
    pub schema_id: u64,
    pub level: u8,
    pub event_name: Arc<str>,
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
        // - Metadata: 4 (length prefix) + 2 * metadata.len()
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
        //   + 2 * event_name.len()
        //   + 4 (row length, u32)
        //   + 4 (Simple Protocol header)
        //   + row_bytes.len()
        //   + 8 (terminator, u64)
        // UTF-16LE uses at most two bytes per UTF-8 byte. Reserve this upper
        // bound so each string only needs to be encoded once.
        let mut estimated_size = 8 + 4 + self.metadata.len() * 2;
        estimated_size += self
            .schemas
            .iter()
            .map(|s| 2 + 8 + 16 + 4 + s.schema.as_bytes().len() + 8)
            .sum::<usize>();
        estimated_size += self
            .events
            .iter()
            .map(|e| {
                let row_len = {
                    4 + &e.row.len() // SP header (4), row_bytes
                };
                2 + 8 + 1 + 2 + 4 + e.event_name.len() * 2 + row_len + 8
            })
            .sum::<usize>();

        let mut buf = Vec::with_capacity(estimated_size);

        // HEADER
        buf.extend_from_slice(&self.version.to_le_bytes());
        buf.extend_from_slice(&self.format.to_le_bytes());

        // METADATA (len, UTF-16LE bytes)
        let metadata_length_offset = buf.len();
        buf.extend_from_slice(&0u32.to_le_bytes());
        let metadata_start = buf.len();
        write_utf16le(&mut buf, &self.metadata);
        let metadata_length = (buf.len() - metadata_start) as u32;
        buf[metadata_length_offset..metadata_start].copy_from_slice(&metadata_length.to_le_bytes());

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
        for event in &self.events {
            buf.extend_from_slice(&2u16.to_le_bytes()); // entity type 2
            buf.extend_from_slice(&event.schema_id.to_le_bytes());
            buf.push(event.level);

            // event name (UTF-16LE, prefixed with u16 len in bytes)
            let name_length_offset = buf.len();
            buf.extend_from_slice(&0u16.to_le_bytes());
            let name_start = buf.len();
            write_utf16le(&mut buf, &event.event_name);
            let name_length = (buf.len() - name_start) as u16; // TODO - check for overflow
            buf[name_length_offset..name_start].copy_from_slice(&name_length.to_le_bytes());

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
    use md5::{Digest as _, Md5};
    use std::borrow::Cow;

    //Helper to calculate MD5 hash, returns [u8;16]
    fn md5_bytes(data: &[u8]) -> [u8; 16] {
        Md5::digest(data).into()
    }

    #[test]
    fn test_empty_central_blob_encoding() {
        let blob = CentralBlob {
            version: 1,
            format: 2,
            metadata: String::new(),
            schemas: Vec::new(),
            events: Vec::new(),
        };

        assert_eq!(blob.to_bytes(), [1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn test_central_blob_utf16_encoding() {
        // Cover empty strings, ASCII, BMP characters, surrogate pairs, and NUL.
        let cases: &[(&str, &[u8])] = &[
            ("", &[]),
            ("log", &[0x6c, 0, 0x6f, 0, 0x67, 0]),
            ("中", &[0x2d, 0x4e]),
            ("😀", &[0x3d, 0xd8, 0, 0xde]),
            ("A\0中😀", &[0x41, 0, 0, 0, 0x2d, 0x4e, 0x3d, 0xd8, 0, 0xde]),
        ];

        for &(metadata, metadata_bytes) in cases {
            let blob = CentralBlob {
                version: 1,
                format: 2,
                metadata: metadata.to_owned(),
                schemas: Vec::new(),
                events: cases
                    .iter()
                    .map(|(name, _)| CentralEventEntry {
                        schema_id: 7,
                        level: 4,
                        event_name: Arc::from(*name),
                        row: vec![0xaa, 0xbb],
                    })
                    .collect(),
            };

            let mut expected = vec![1, 0, 0, 0, 2, 0, 0, 0];
            expected.extend_from_slice(&(metadata_bytes.len() as u32).to_le_bytes());
            expected.extend_from_slice(metadata_bytes);
            for &(_, name_bytes) in cases {
                expected.extend_from_slice(&[2, 0, 7, 0, 0, 0, 0, 0, 0, 0, 4]);
                expected.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
                expected.extend_from_slice(name_bytes);
                // Row length, Simple Protocol header, row data, and terminator.
                expected.extend_from_slice(&[
                    6, 0, 0, 0, 0x53, 0x50, 1, 0, 0xaa, 0xbb, 0xde, 0xc0, 0xad, 0xde, 0xde, 0xc0,
                    0xad, 0xde,
                ]);
            }
            assert_eq!(blob.to_bytes(), expected, "metadata: {metadata:?}");
        }
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
        let schema_obj = BondEncodedSchema::from_fields("TestStruct", "test.namespace", &fields);
        let schema_bytes = schema_obj.as_bytes().to_vec();
        let schema_md5 = md5_bytes(&schema_bytes);
        let schema_id = 1234u64;

        let schema = CentralSchemaEntry {
            id: schema_id,
            md5: schema_md5,
            schema: schema_obj,
            fields,
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
            event_name: Arc::from("eventname"),
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
