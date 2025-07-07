// bond_encoder.rs - Pure Rust Bond encoder for dynamic OTLP schemas

use std::borrow::Cow;
use std::io::{Result, Write};

/// Bond data types
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
#[allow(non_camel_case_types)] // Allow C-style naming for clarity with Bond protocol
#[allow(dead_code)] // These represent all possible Bond types, not all are used yet
pub(crate) enum BondDataType {
    BT_STOP = 0,
    BT_STOP_BASE = 1,
    BT_BOOL = 2,
    BT_UINT8 = 3,
    BT_UINT16 = 4,
    BT_UINT32 = 5,
    BT_UINT64 = 6,
    BT_FLOAT = 7,
    BT_DOUBLE = 8,
    BT_STRING = 9,
    BT_STRUCT = 10,
    BT_LIST = 11,
    BT_SET = 12,
    BT_MAP = 13,
    BT_INT8 = 14,
    BT_INT16 = 15,
    BT_INT32 = 16,
    BT_INT64 = 17,
    BT_WSTRING = 18,
    BT_UNAVAILABLE = 127,
}

/// Bond protocol writer for encoding values to buffers
pub(crate) struct BondWriter;

// Trait for types that can be converted to little-endian bytes
/// This is automatically implemented for all primitive numeric types
trait ToLeBytes {
    type ByteArray: AsRef<[u8]>;
    fn to_le_bytes(self) -> Self::ByteArray;
}

// Implement for all standard numeric types
macro_rules! impl_to_le_bytes {
    ($($t:ty),*) => {
        $(
            impl ToLeBytes for $t {
                type ByteArray = [u8; std::mem::size_of::<$t>()];
                fn to_le_bytes(self) -> Self::ByteArray {
                    <$t>::to_le_bytes(self)
                }
            }
        )*
    };
}

impl_to_le_bytes!(i8, i16, i32, i64, u8, u16, u32, u64, f32, f64);

impl BondWriter {
    /// Write any primitive numeric type to buffer in little-endian format
    /// Works for i32, i64, f32, f64, u32, u64, etc.
    fn write_numeric<T>(buffer: &mut Vec<u8>, value: T)
    where
        T: ToLeBytes,
    {
        buffer.extend_from_slice(value.to_le_bytes().as_ref());
    }

    /// Write a string value to buffer (Bond BT_STRING format)
    pub fn write_string(buffer: &mut Vec<u8>, s: &str) {
        let bytes = s.as_bytes();
        //TODO - check if the length is less than 2^32-1
        Self::write_numeric(buffer, bytes.len() as u32);
        buffer.extend_from_slice(bytes);
    }

    /// Write an int32 value to buffer (Bond BT_INT32 format)
    pub fn write_int32(buffer: &mut Vec<u8>, value: i32) {
        Self::write_numeric(buffer, value);
    }

    /// Write an int64 value to buffer (Bond BT_INT64 format)
    pub fn write_int64(buffer: &mut Vec<u8>, value: i64) {
        Self::write_numeric(buffer, value);
    }

    /// Write a float value to buffer (Bond BT_FLOAT format)
    pub fn write_float(buffer: &mut Vec<u8>, value: f32) {
        Self::write_numeric(buffer, value);
    }

    /// Write a double value to buffer (Bond BT_DOUBLE format)
    pub fn write_double(buffer: &mut Vec<u8>, value: f64) {
        Self::write_numeric(buffer, value);
    }

    /// Write a boolean value to buffer (Bond BT_BOOL format)
    /// In Simple Binary protocol, booleans are encoded as single bytes
    pub fn write_bool(buffer: &mut Vec<u8>, value: bool) {
        buffer.push(if value { 1u8 } else { 0u8 });
    }

    /// Write a WSTRING value to buffer (Bond BT_WSTRING format)
    /// Character count prefix + UTF-16LE bytes
    pub fn write_wstring(buffer: &mut Vec<u8>, s: &str) {
        let utf16_bytes: Vec<u8> = s.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();

        // Character count (not byte count)
        // TODO - check if the length is less than 2^32-1
        // TODO - check if length is number of bytes, or number of UTF-16 code units
        Self::write_numeric(buffer, s.len() as u32);
        buffer.extend_from_slice(&utf16_bytes);
    }
}

/// Field definition for dynamic schemas
#[derive(Clone, Debug)]
pub(crate) struct FieldDef {
    pub name: Cow<'static, str>,
    pub field_id: u16,
    pub type_id: u8,
}

/// Schema definition that can be built dynamically
#[derive(Clone)]
pub(crate) struct DynamicSchema {
    pub struct_name: String,
    pub qualified_name: String,
    pub fields: Vec<FieldDef>,
}

impl DynamicSchema {
    pub(crate) fn new(name: &str, namespace: &str, fields: Vec<FieldDef>) -> Self {
        Self {
            struct_name: name.to_string(),
            qualified_name: format!("{namespace}.{name}"),
            fields,
        }
    }

    /// Encode the schema to Bond format
    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        let mut schema_bytes = Vec::new();

        // Write header
        schema_bytes.write_all(&[0x53, 0x50])?; // 'S','P'
        schema_bytes.write_all(&[0x01, 0x00])?; // Version 1
        schema_bytes.write_all(&1u32.to_le_bytes())?; // num structs

        // Write struct definition
        write_bond_string(&mut schema_bytes, &self.struct_name)?;
        write_bond_string(&mut schema_bytes, &self.qualified_name)?;
        schema_bytes.write_all(&0u32.to_le_bytes())?; // attributes

        // Modifier - 0 for Optional
        schema_bytes.write_all(&[0u8])?;

        // Default values
        schema_bytes.write_all(&0u64.to_le_bytes())?; // default_uint
        schema_bytes.write_all(&0i64.to_le_bytes())?; // default_int
        schema_bytes.write_all(&0f64.to_le_bytes())?; // default_double
        schema_bytes.write_all(&0u32.to_le_bytes())?; // default_string
        schema_bytes.write_all(&0u32.to_le_bytes())?; // default_wstring
        schema_bytes.write_all(&[0u8])?; // default_nothing

        // Base def
        schema_bytes.write_all(&0u32.to_le_bytes())?;

        // 3 bytes of zeros before num_fields
        schema_bytes.write_all(&[0u8; 3])?;

        // Number of fields
        schema_bytes.write_all(&(self.fields.len() as u32).to_le_bytes())?;

        // Write field definitions
        for (i, field) in self.fields.iter().enumerate() {
            let is_last = i == self.fields.len() - 1;
            write_field_def(&mut schema_bytes, field, is_last)?;
        }

        // Padding to align to 8 bytes
        schema_bytes.write_all(&[0u8; 8])?;

        // Root type typedef
        schema_bytes.write_all(&[BondDataType::BT_STRUCT as u8])?;
        schema_bytes.write_all(&0u16.to_le_bytes())?; // struct index 0
        schema_bytes.write_all(&[0u8])?; // element
        schema_bytes.write_all(&[0u8])?; // key
        schema_bytes.write_all(&[0u8])?; // bonded = false

        // Final padding
        schema_bytes.write_all(&[0u8; 9])?;

        Ok(schema_bytes)
    }
}

fn write_bond_string<W: Write>(writer: &mut W, s: &str) -> Result<()> {
    let bytes = s.as_bytes();
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(bytes)?;
    Ok(())
}

fn write_field_def<W: Write>(writer: &mut W, field: &FieldDef, is_last: bool) -> Result<()> {
    // Field name
    write_bond_string(writer, field.name.as_ref())?;

    // Empty qualified name
    write_bond_string(writer, "")?;

    // Attributes
    writer.write_all(&0u32.to_le_bytes())?;

    // Modifier
    writer.write_all(&[0u8])?;

    // Default values (all zeros for primitives)
    writer.write_all(&0u64.to_le_bytes())?; // default_uint
    writer.write_all(&0i64.to_le_bytes())?; // default_int
    writer.write_all(&0f64.to_le_bytes())?; // default_double
    writer.write_all(&0u32.to_le_bytes())?; // default_string
    writer.write_all(&0u32.to_le_bytes())?; // default_wstring
    writer.write_all(&[0u8])?; // default_nothing

    // Add 3 bytes of padding before field ID
    writer.write_all(&[0u8; 3])?;

    // Field ID
    writer.write_all(&field.field_id.to_le_bytes())?;

    // Type
    writer.write_all(&[field.type_id])?;

    // Additional type info (all zeros for primitives)
    writer.write_all(&0u16.to_le_bytes())?; // struct_def
    writer.write_all(&[0u8])?; // element
    writer.write_all(&[0u8])?; // key
    writer.write_all(&[0u8])?; // bonded_type
    writer.write_all(&[0u8])?; // default_value_present

    // Add 8 bytes padding after each field except the last one
    if !is_last {
        writer.write_all(&[0u8; 8])?;
    }

    Ok(())
}

/// Encode a payload with dynamic fields
#[allow(dead_code)] // May be used in future
pub(crate) fn encode_dynamic_payload<W: Write>(
    writer: &mut W,
    fields: &[FieldDef],
    values: &[(&str, Vec<u8>)], // field_name -> encoded value
) -> Result<()> {
    // Write Simple Binary header
    writer.write_all(&[0x53, 0x50])?; // 'S','P'
    writer.write_all(&[0x01, 0x00])?; // Version 1

    // Create a map for quick lookup
    let value_map: std::collections::HashMap<&str, &[u8]> =
        values.iter().map(|(k, v)| (*k, v.as_slice())).collect();

    // Write values in field order
    for field in fields {
        if let Some(value_bytes) = value_map.get(field.name.as_ref()) {
            writer.write_all(value_bytes)?;
        } else {
            // Write default value based on type
            match field.type_id {
                2 => writer.write_all(&[0u8])?,              // bool - single byte
                7 => writer.write_all(&0f32.to_le_bytes())?, // float
                8 => writer.write_all(&0f64.to_le_bytes())?, // double
                9 | 18 => writer.write_all(&0u32.to_le_bytes())?, // empty string
                16 => writer.write_all(&0i32.to_le_bytes())?, // int32
                17 => writer.write_all(&0i64.to_le_bytes())?, // int64
                _ => {}                                      // Handle other types as needed
            }
        }
    }

    Ok(())
}

pub(crate) struct BondEncodedSchema {
    schema: DynamicSchema,
    encoded_bytes: Vec<u8>,
}

impl BondEncodedSchema {
    pub(crate) fn from_fields(name: &str, namespace: &str, fields: Vec<FieldDef>) -> Self {
        let schema = DynamicSchema::new(name, namespace, fields); //"OtlpLogRecord", "telemetry");

        let encoded_bytes = schema.encode().expect("Schema encoding failed");

        Self {
            schema,
            encoded_bytes,
        }
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.encoded_bytes
    }
}

impl Clone for BondEncodedSchema {
    fn clone(&self) -> Self {
        Self {
            schema: self.schema.clone(),
            encoded_bytes: self.encoded_bytes.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    #[test]
    fn test_dynamic_schema() {
        // Create fields directly as FieldDef
        let fields = vec![
            FieldDef {
                name: Cow::Borrowed("field1"),
                type_id: BondDataType::BT_DOUBLE as u8,
                field_id: 1,
            },
            FieldDef {
                name: Cow::Borrowed("field2"),
                type_id: BondDataType::BT_STRING as u8,
                field_id: 2,
            },
            FieldDef {
                name: Cow::Borrowed("field3"),
                type_id: BondDataType::BT_INT32 as u8,
                field_id: 3,
            },
        ];

        let schema = DynamicSchema::new("TestStruct", "test.namespace", fields);
        let encoded = schema.encode().unwrap();
        assert!(!encoded.is_empty());
    }

    #[test]
    fn test_pure_rust_encoder_schema() {
        let fields = vec![
            FieldDef {
                name: Cow::Borrowed("timestamp"),
                type_id: BondDataType::BT_STRING as u8,
                field_id: 1,
            },
            FieldDef {
                name: Cow::Borrowed("severity"),
                type_id: BondDataType::BT_INT32 as u8,
                field_id: 2,
            },
            FieldDef {
                name: Cow::Borrowed("message"),
                type_id: BondDataType::BT_STRING as u8,
                field_id: 3,
            },
        ];

        let schema = BondEncodedSchema::from_fields("OtlpLogRecord", "telemetry", fields);
        let bytes = schema.as_bytes();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn test_field_def_with_owned_strings() {
        // Test that FieldDef works with owned strings too
        let dynamic_field_name = format!("dynamic_{}", 123);
        let fields = vec![
            FieldDef {
                name: Cow::Owned(dynamic_field_name),
                type_id: BondDataType::BT_STRING as u8,
                field_id: 1,
            },
            FieldDef {
                name: Cow::Borrowed("static_field"),
                type_id: BondDataType::BT_INT32 as u8,
                field_id: 2,
            },
        ];

        let schema = BondEncodedSchema::from_fields("TestStruct", "test.namespace", fields);
        let bytes = schema.as_bytes();
        assert!(!bytes.is_empty());
    }
}
