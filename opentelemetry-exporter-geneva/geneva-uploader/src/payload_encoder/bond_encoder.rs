// bond_encoder.rs - Pure Rust Bond encoder for dynamic OTLP schemas

use std::borrow::Cow;
use std::io::{Result, Write};
use std::sync::Arc;

/// Bond data types
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
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
pub(crate) trait ToLeBytes {
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

impl_to_le_bytes!(i32, i64, u32, f64);

impl BondWriter {
    /// Write primitive numeric type to buffer in little-endian format
    /// Works for i32, i64, u32, f64.
    #[inline]
    pub fn write_numeric<T>(buffer: &mut Vec<u8>, value: T)
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

    /// Write a boolean value to buffer (Bond BT_BOOL format)
    /// In Simple Binary protocol, booleans are encoded as single bytes
    pub fn write_bool(buffer: &mut Vec<u8>, value: bool) {
        buffer.push(if value { 1u8 } else { 0u8 });
    }

    /// Write a WSTRING value to buffer (Bond BT_WSTRING format)
    /// Character count prefix + UTF-16LE bytes
    #[allow(dead_code)] // May be used in future, for now used in tests
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
    pub type_id: BondDataType,
}

/// Schema definition that can be built dynamically
#[derive(Clone)]
pub(crate) struct DynamicSchema<'a> {
    pub struct_name: String,
    pub qualified_name: String,
    pub fields: &'a [FieldDef],
}

impl<'a> DynamicSchema<'a> {
    pub(crate) fn new(name: &str, namespace: &str, fields: &'a [FieldDef]) -> Self {
        Self {
            struct_name: name.to_string(),
            qualified_name: format!("{namespace}.{name}"),
            fields,
        }
    }

    /// Calculate the fixed overhead size for Bond schema encoding
    const fn calculate_fixed_overhead() -> usize {
        // Bond header: "SP" + version
        let mut size = 4; // [0x53, 0x50, 0x01, 0x00]

        // Number of structs
        size += 4; // 1u32

        // Struct definition header (excluding variable name lengths)
        size += 4; // attributes (0u32)
        size += 1; // modifier (0u8)

        // Default values block
        size += 8; // default_uint (u64)
        size += 8; // default_int (i64)
        size += 8; // default_double (f64)
        size += 4; // default_string (u32)
        size += 4; // default_wstring (u32)
        size += 1; // default_nothing (u8)

        // Base def
        size += 4; // 0u32

        // Field count header
        size += 3; // 3 zero bytes
        size += 4; // field count (u32)

        // Post-fields padding
        size += 8; // alignment padding

        // Root typedef
        size += 1; // BondDataType::BT_STRUCT
        size += 2; // struct index (u16)
        size += 1; // element (u8)
        size += 1; // key (u8)
        size += 1; // bonded (u8)

        // Final padding
        size += 9; // 9 zero bytes

        size
    }

    /// Calculate the fixed overhead per field (excluding variable field name length)
    const fn calculate_per_field_fixed_overhead() -> usize {
        // Empty qualified name
        let mut size = 4; // 0u32 for empty string

        // Field attributes and data
        size += 4; // attributes (0u32)
        size += 1; // modifier (0u8)

        // Default values block for field
        size += 8; // default_uint (u64)
        size += 8; // default_int (i64)
        size += 8; // default_double (f64)
        size += 4; // default_string (u32)
        size += 4; // default_wstring (u32)
        size += 1; // default_nothing (u8)

        // Field metadata
        size += 3; // 3 padding bytes
        size += 2; // field_id (u16)
        size += 1; // type_id (u8)

        // Additional type info
        size += 2; // struct_def (u16)
        size += 1; // element (u8)
        size += 1; // key (u8)
        size += 1; // bonded_type (u8)
        size += 1; // default_value_present (u8)

        size
    }

    /// Calculate the exact size of the encoded schema in bytes
    pub(crate) fn calculate_exact_encoded_size(&self) -> usize {
        // Start with fixed overhead
        let mut size = Self::calculate_fixed_overhead();

        // Add variable struct name lengths
        size += 4 + self.struct_name.len(); // struct_name length + bytes
        size += 4 + self.qualified_name.len(); // qualified_name length + bytes

        // Add field-specific sizes
        for (i, field) in self.fields.iter().enumerate() {
            let is_last = i == self.fields.len() - 1;

            // Fixed overhead per field
            size += Self::calculate_per_field_fixed_overhead();

            // Variable field name
            size += 4 + field.name.len(); // name length + bytes

            // Padding after each field except the last
            if !is_last {
                size += 8;
            }
        }

        size
    }

    /// Encode the schema to Bond format
    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        let exact_size = self.calculate_exact_encoded_size();
        let mut schema_bytes = Vec::with_capacity(exact_size);

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
    writer.write_all(&[field.type_id as u8])?;

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
                BondDataType::BT_BOOL => writer.write_all(&[0u8])?, // bool - single byte
                BondDataType::BT_FLOAT => writer.write_all(&0f32.to_le_bytes())?, // float
                BondDataType::BT_DOUBLE => writer.write_all(&0f64.to_le_bytes())?, // double
                BondDataType::BT_STRING | BondDataType::BT_WSTRING => {
                    writer.write_all(&0u32.to_le_bytes())?
                } // empty string
                BondDataType::BT_INT32 => writer.write_all(&0i32.to_le_bytes())?, // int32
                BondDataType::BT_INT64 => writer.write_all(&0i64.to_le_bytes())?, // int64
                _ => {}                                             // Handle other types as needed
            }
        }
    }

    Ok(())
}

/// Arc-wrapped schema data to avoid expensive cloning
type BondSchemaData = Vec<u8>;
pub(crate) struct BondEncodedSchema {
    data: Arc<BondSchemaData>,
}

impl BondEncodedSchema {
    pub(crate) fn from_fields(name: &str, namespace: &str, fields: &[FieldDef]) -> Self {
        let schema = DynamicSchema::new(name, namespace, fields);
        let encoded_bytes = schema.encode().expect("Schema encoding failed");

        Self {
            data: Arc::new(encoded_bytes),
        }
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.data
    }
}

impl Clone for BondEncodedSchema {
    fn clone(&self) -> Self {
        Self {
            data: Arc::clone(&self.data),
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
                type_id: BondDataType::BT_DOUBLE,
                field_id: 1,
            },
            FieldDef {
                name: Cow::Borrowed("field2"),
                type_id: BondDataType::BT_STRING,
                field_id: 2,
            },
            FieldDef {
                name: Cow::Borrowed("field3"),
                type_id: BondDataType::BT_INT32,
                field_id: 3,
            },
        ];

        let schema = DynamicSchema::new("TestStruct", "test.namespace", &fields);
        let encoded = schema.encode().unwrap();
        assert!(!encoded.is_empty());
    }

    #[test]
    fn test_pure_rust_encoder_schema() {
        let fields = vec![
            FieldDef {
                name: Cow::Borrowed("timestamp"),
                type_id: BondDataType::BT_STRING,
                field_id: 1,
            },
            FieldDef {
                name: Cow::Borrowed("severity"),
                type_id: BondDataType::BT_INT32,
                field_id: 2,
            },
            FieldDef {
                name: Cow::Borrowed("message"),
                type_id: BondDataType::BT_STRING,
                field_id: 3,
            },
        ];

        let schema = BondEncodedSchema::from_fields("OtlpLogRecord", "telemetry", &fields);
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
                type_id: BondDataType::BT_STRING,
                field_id: 1,
            },
            FieldDef {
                name: Cow::Borrowed("static_field"),
                type_id: BondDataType::BT_INT32,
                field_id: 2,
            },
        ];

        let schema = BondEncodedSchema::from_fields("TestStruct", "test.namespace", &fields);
        let bytes = schema.as_bytes();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn test_schema_exact_size_calculation() {
        // Test that the exact size calculation matches the actual encoded size
        // This validates that pre-allocation is precise and no reallocations occur

        // Test with different field counts and varying field name lengths
        let test_cases = vec![
            (0, "no fields"),
            (1, "single field"),
            (5, "few fields"),
            (10, "medium fields"),
            (20, "many fields"),
        ];

        for (field_count, description) in test_cases {
            // Create fields with varying name lengths to test the calculation accuracy
            let fields: Vec<FieldDef> = (0..field_count)
                .map(|i| FieldDef {
                    name: Cow::Owned(format!("field_with_long_name_{i}")),
                    type_id: BondDataType::BT_STRING,
                    field_id: i as u16 + 1,
                })
                .collect();

            let schema = DynamicSchema::new("TestStruct", "test.namespace", &fields);
            let exact_size = schema.calculate_exact_encoded_size();
            let encoded = schema.encode().unwrap();

            // Verify encoding succeeded
            assert!(!encoded.is_empty(), "Encoding failed for {description}");

            // The exact calculation should match the actual encoded size precisely
            let actual_size = encoded.len();
            assert_eq!(
                exact_size, actual_size,
                "Exact size calculation {exact_size} does not match actual size {actual_size} for {description}"
            );
        }
    }
}
