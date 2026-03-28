use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::{Map, Number, Value};
use std::io::{Cursor, Read};

const BOND_HEADER: [u8; 4] = [0x53, 0x50, 0x01, 0x00];

#[derive(Clone, Copy, Debug, Serialize)]
#[repr(u8)]
pub(crate) enum BondDataType {
    Stop = 0,
    StopBase = 1,
    Bool = 2,
    Uint8 = 3,
    Uint16 = 4,
    Uint32 = 5,
    Uint64 = 6,
    Float = 7,
    Double = 8,
    String = 9,
    Struct = 10,
    List = 11,
    Set = 12,
    Map = 13,
    Int8 = 14,
    Int16 = 15,
    Int32 = 16,
    Int64 = 17,
    Wstring = 18,
}

impl BondDataType {
    fn from_u8(value: u8) -> Result<Self> {
        let ty = match value {
            0 => Self::Stop,
            1 => Self::StopBase,
            2 => Self::Bool,
            3 => Self::Uint8,
            4 => Self::Uint16,
            5 => Self::Uint32,
            6 => Self::Uint64,
            7 => Self::Float,
            8 => Self::Double,
            9 => Self::String,
            10 => Self::Struct,
            11 => Self::List,
            12 => Self::Set,
            13 => Self::Map,
            14 => Self::Int8,
            15 => Self::Int16,
            16 => Self::Int32,
            17 => Self::Int64,
            18 => Self::Wstring,
            _ => bail!("unsupported Bond type id {value}"),
        };
        Ok(ty)
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SchemaField {
    pub(crate) name: String,
    pub(crate) field_id: u16,
    pub(crate) type_id: BondDataType,
}

#[derive(Clone, Debug)]
pub(crate) struct DecodedSchema {
    pub(crate) struct_name: String,
    pub(crate) qualified_name: String,
    pub(crate) fields: Vec<SchemaField>,
}

pub(crate) fn decode_schema(bytes: &[u8]) -> Result<DecodedSchema> {
    let mut cursor = Cursor::new(bytes);
    expect_bytes(&mut cursor, &BOND_HEADER)?;

    let num_structs = read_u32(&mut cursor)?;
    if num_structs != 1 {
        bail!("expected 1 struct in schema, found {num_structs}");
    }

    let struct_name = read_bond_string(&mut cursor)?;
    let qualified_name = read_bond_string(&mut cursor)?;

    skip(&mut cursor, 4 + 1 + 8 + 8 + 8 + 4 + 4 + 1 + 4 + 3)?;
    let field_count = read_u32(&mut cursor)? as usize;

    let mut fields = Vec::with_capacity(field_count);
    for index in 0..field_count {
        let name = read_bond_string(&mut cursor)?;
        let _qualified = read_bond_string(&mut cursor)?;
        skip(&mut cursor, 4 + 1 + 8 + 8 + 8 + 4 + 4 + 1 + 3)?;
        let field_id = read_u16(&mut cursor)?;
        let type_id = BondDataType::from_u8(read_u8(&mut cursor)?)?;
        skip(&mut cursor, 2 + 1 + 1 + 1 + 1)?;

        fields.push(SchemaField {
            name,
            field_id,
            type_id,
        });

        if index + 1 != field_count {
            skip(&mut cursor, 8)?;
        }
    }

    skip(&mut cursor, 8 + 1 + 2 + 1 + 1 + 1 + 9)?;

    if cursor.position() as usize != bytes.len() {
        bail!(
            "schema parser stopped at {}, expected {}",
            cursor.position(),
            bytes.len()
        );
    }

    Ok(DecodedSchema {
        struct_name,
        qualified_name,
        fields,
    })
}

pub(crate) fn decode_row(schema: &DecodedSchema, row: &[u8]) -> Result<Value> {
    let mut cursor = Cursor::new(row);
    let mut object = Map::with_capacity(schema.fields.len());

    for field in &schema.fields {
        let value = match field.type_id {
            BondDataType::Bool => Value::Bool(read_u8(&mut cursor)? != 0),
            BondDataType::Uint8 => Value::Number(Number::from(read_u8(&mut cursor)?)),
            BondDataType::Uint16 => Value::Number(Number::from(read_u16(&mut cursor)?)),
            BondDataType::Uint32 => Value::Number(Number::from(read_u32(&mut cursor)?)),
            BondDataType::Uint64 => Value::Number(Number::from(read_u64(&mut cursor)?)),
            BondDataType::Int8 => Value::Number(Number::from(read_i8(&mut cursor)?)),
            BondDataType::Int16 => Value::Number(Number::from(read_i16(&mut cursor)?)),
            BondDataType::Int32 => Value::Number(Number::from(read_i32(&mut cursor)?)),
            BondDataType::Int64 => Value::Number(Number::from(read_i64(&mut cursor)?)),
            BondDataType::Float => {
                let value = read_f32(&mut cursor)?;
                number_from_f64(value as f64)?
            }
            BondDataType::Double => {
                let value = read_f64(&mut cursor)?;
                number_from_f64(value)?
            }
            BondDataType::String => Value::String(read_bond_string(&mut cursor)?),
            BondDataType::Wstring => Value::String(read_bond_wstring(&mut cursor)?),
            unsupported => bail!("unsupported row field type {:?}", unsupported),
        };
        object.insert(field.name.clone(), value);
    }

    if cursor.position() as usize != row.len() {
        bail!(
            "row parser stopped at {}, expected {}",
            cursor.position(),
            row.len()
        );
    }

    Ok(Value::Object(object))
}

fn number_from_f64(value: f64) -> Result<Value> {
    Number::from_f64(value)
        .map(Value::Number)
        .ok_or_else(|| anyhow!("non-finite float value"))
}

fn expect_bytes(cursor: &mut Cursor<&[u8]>, expected: &[u8]) -> Result<()> {
    let mut actual = vec![0u8; expected.len()];
    cursor.read_exact(&mut actual)?;
    if actual != expected {
        bail!("unexpected bytes: expected {expected:?}, got {actual:?}");
    }
    Ok(())
}

fn skip(cursor: &mut Cursor<&[u8]>, len: usize) -> Result<()> {
    let next = cursor.position() as usize + len;
    if next > cursor.get_ref().len() {
        bail!("unexpected EOF while skipping {len} bytes");
    }
    cursor.set_position(next as u64);
    Ok(())
}

fn read_bond_string(cursor: &mut Cursor<&[u8]>) -> Result<String> {
    let len = read_u32(cursor)? as usize;
    let bytes = read_exact(cursor, len)?;
    String::from_utf8(bytes.to_vec()).context("invalid UTF-8 string")
}

fn read_bond_wstring(cursor: &mut Cursor<&[u8]>) -> Result<String> {
    let char_count = read_u32(cursor)? as usize;
    let bytes = read_exact(cursor, char_count * 2)?;
    let utf16 = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    String::from_utf16(&utf16).context("invalid UTF-16 string")
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

fn read_i8(cursor: &mut Cursor<&[u8]>) -> Result<i8> {
    Ok(read_u8(cursor)? as i8)
}

fn read_u16(cursor: &mut Cursor<&[u8]>) -> Result<u16> {
    let bytes = read_exact(cursor, 2)?;
    Ok(u16::from_le_bytes(bytes.try_into().expect("2-byte slice")))
}

fn read_i16(cursor: &mut Cursor<&[u8]>) -> Result<i16> {
    let bytes = read_exact(cursor, 2)?;
    Ok(i16::from_le_bytes(bytes.try_into().expect("2-byte slice")))
}

fn read_u32(cursor: &mut Cursor<&[u8]>) -> Result<u32> {
    let bytes = read_exact(cursor, 4)?;
    Ok(u32::from_le_bytes(bytes.try_into().expect("4-byte slice")))
}

fn read_i32(cursor: &mut Cursor<&[u8]>) -> Result<i32> {
    let bytes = read_exact(cursor, 4)?;
    Ok(i32::from_le_bytes(bytes.try_into().expect("4-byte slice")))
}

fn read_u64(cursor: &mut Cursor<&[u8]>) -> Result<u64> {
    let bytes = read_exact(cursor, 8)?;
    Ok(u64::from_le_bytes(bytes.try_into().expect("8-byte slice")))
}

fn read_i64(cursor: &mut Cursor<&[u8]>) -> Result<i64> {
    let bytes = read_exact(cursor, 8)?;
    Ok(i64::from_le_bytes(bytes.try_into().expect("8-byte slice")))
}

fn read_f32(cursor: &mut Cursor<&[u8]>) -> Result<f32> {
    let bytes = read_exact(cursor, 4)?;
    Ok(f32::from_le_bytes(bytes.try_into().expect("4-byte slice")))
}

fn read_f64(cursor: &mut Cursor<&[u8]>) -> Result<f64> {
    let bytes = read_exact(cursor, 8)?;
    Ok(f64::from_le_bytes(bytes.try_into().expect("8-byte slice")))
}
