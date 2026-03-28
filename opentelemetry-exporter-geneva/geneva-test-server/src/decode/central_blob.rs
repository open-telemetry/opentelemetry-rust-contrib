use crate::decode::bond::{decode_row, decode_schema, DecodedSchema, SchemaField};
use anyhow::{anyhow, bail, Context, Result};
use hex::encode as hex_encode;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Cursor;

const TERMINATOR: u64 = 0xdeadc0dedeadc0de;
const SIMPLE_PROTOCOL_HEADER: [u8; 4] = [0x53, 0x50, 0x01, 0x00];

#[derive(Debug)]
pub(crate) struct DecodedBlob {
    pub(crate) metadata: String,
    pub(crate) schemas: Vec<DecodedSchemaEntry>,
    pub(crate) events: Vec<DecodedEventEntry>,
}

#[derive(Debug)]
pub(crate) struct DecodedSchemaEntry {
    pub(crate) schema_id: u64,
    pub(crate) md5: String,
    pub(crate) schema: DecodedSchema,
}

#[derive(Debug)]
pub(crate) struct DecodedEventEntry {
    pub(crate) schema_id: u64,
    pub(crate) level: u8,
    pub(crate) event_name: String,
    pub(crate) payload: Value,
}

pub(crate) fn decode_central_blob(bytes: &[u8]) -> Result<DecodedBlob> {
    let mut cursor = Cursor::new(bytes);
    let _version = read_u32(&mut cursor)?;
    let _format = read_u32(&mut cursor)?;
    let metadata = read_utf16le_string_u32(&mut cursor)?;

    let mut schema_map = HashMap::new();
    let mut schemas = Vec::new();
    let mut events = Vec::new();

    while (cursor.position() as usize) < bytes.len() {
        let entity_type = read_u16(&mut cursor)?;
        match entity_type {
            0 => {
                let schema_id = read_u64(&mut cursor)?;
                let md5 = read_exact(&mut cursor, 16)?;
                let schema_len = read_u32(&mut cursor)? as usize;
                let schema_bytes = read_exact(&mut cursor, schema_len)?;
                expect_terminator(&mut cursor)?;

                let schema = decode_schema(schema_bytes)
                    .with_context(|| format!("failed to decode schema {schema_id}"))?;
                let entry = DecodedSchemaEntry {
                    schema_id,
                    md5: hex_encode(md5),
                    schema: schema.clone(),
                };
                schema_map.insert(schema_id, schema);
                schemas.push(entry);
            }
            2 => {
                let schema_id = read_u64(&mut cursor)?;
                let level = read_u8(&mut cursor)?;
                let event_name = read_utf16le_string_u16(&mut cursor)?;
                let row_len = read_u32(&mut cursor)? as usize;
                let row_bytes = read_exact(&mut cursor, row_len)?;
                expect_terminator(&mut cursor)?;

                if !row_bytes.starts_with(&SIMPLE_PROTOCOL_HEADER) {
                    bail!("missing row simple protocol header for schema {schema_id}");
                }
                let schema = schema_map
                    .get(&schema_id)
                    .ok_or_else(|| anyhow!("event references unknown schema {schema_id}"))?;
                let payload = decode_row(schema, &row_bytes[SIMPLE_PROTOCOL_HEADER.len()..])
                    .with_context(|| format!("failed to decode row for schema {schema_id}"))?;
                events.push(DecodedEventEntry {
                    schema_id,
                    level,
                    event_name,
                    payload,
                });
            }
            other => bail!("unsupported central blob entity type {other}"),
        }
    }

    Ok(DecodedBlob {
        metadata,
        schemas,
        events,
    })
}

pub(crate) fn schema_fields_json(fields: &[SchemaField]) -> Value {
    serde_json::to_value(fields).unwrap_or(Value::Null)
}

fn expect_terminator(cursor: &mut Cursor<&[u8]>) -> Result<()> {
    let value = read_u64(cursor)?;
    if value != TERMINATOR {
        bail!("invalid central blob terminator {value:#x}");
    }
    Ok(())
}

fn read_utf16le_string_u32(cursor: &mut Cursor<&[u8]>) -> Result<String> {
    let len = read_u32(cursor)? as usize;
    read_utf16le_string(cursor, len)
}

fn read_utf16le_string_u16(cursor: &mut Cursor<&[u8]>) -> Result<String> {
    let len = read_u16(cursor)? as usize;
    read_utf16le_string(cursor, len)
}

fn read_utf16le_string(cursor: &mut Cursor<&[u8]>, len: usize) -> Result<String> {
    let bytes = read_exact(cursor, len)?;
    if bytes.len() % 2 != 0 {
        bail!("UTF-16LE string length {len} is not even");
    }
    let utf16 = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    String::from_utf16(&utf16).context("invalid UTF-16LE string")
}

fn read_exact<'a>(cursor: &mut Cursor<&'a [u8]>, len: usize) -> Result<&'a [u8]> {
    let start = cursor.position() as usize;
    let end = start + len;
    let slice = cursor
        .get_ref()
        .get(start..end)
        .ok_or_else(|| anyhow!("unexpected EOF reading {len} bytes"))?;
    cursor.set_position(end as u64);
    Ok(slice)
}

fn read_u8(cursor: &mut Cursor<&[u8]>) -> Result<u8> {
    Ok(read_exact(cursor, 1)?[0])
}

fn read_u16(cursor: &mut Cursor<&[u8]>) -> Result<u16> {
    let bytes = read_exact(cursor, 2)?;
    Ok(u16::from_le_bytes(bytes.try_into().expect("2-byte slice")))
}

fn read_u32(cursor: &mut Cursor<&[u8]>) -> Result<u32> {
    let bytes = read_exact(cursor, 4)?;
    Ok(u32::from_le_bytes(bytes.try_into().expect("4-byte slice")))
}

fn read_u64(cursor: &mut Cursor<&[u8]>) -> Result<u64> {
    let bytes = read_exact(cursor, 8)?;
    Ok(u64::from_le_bytes(bytes.try_into().expect("8-byte slice")))
}
