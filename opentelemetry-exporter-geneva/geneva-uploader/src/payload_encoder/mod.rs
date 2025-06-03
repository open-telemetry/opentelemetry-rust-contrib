pub(crate) mod central_blob;
pub(crate) mod ffi;
pub(crate) mod lz4_chunked_compression;
pub(crate) mod otlp_encoder;

use std::slice;

#[allow(dead_code)]
pub(crate) struct EncoderSchema {
    schema_result: ffi::SchemaResultPtr,
    fields: Vec<(String, u8, u16)>, // (name, type, id)
}

#[allow(dead_code)]
pub(crate) struct EncoderRow {
    bytes: Vec<u8>,
}

impl Clone for EncoderSchema {
    fn clone(&self) -> Self {
        EncoderSchema::from_fields(
            &self
                .fields
                .iter()
                .map(|(n, t, i)| (n.as_str(), *t, *i))
                .collect::<Vec<_>>(),
        )
    }
}

impl Drop for EncoderSchema {
    fn drop(&mut self) {
        unsafe {
            if !self.schema_result.0.is_null() {
                ffi::free_schema_buf_ffi(self.schema_result.0);
            }
        }
    }
}

impl EncoderSchema {
    #[allow(dead_code)]
    pub(crate) fn from_fields(fields: &[(&str, u8, u16)]) -> Self {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(fields.len() as u16).to_le_bytes());
        for (name, typ, id) in fields {
            buf.push(name.len() as u8);
            buf.extend_from_slice(name.as_bytes());
            buf.push(*typ);
            buf.extend_from_slice(&id.to_le_bytes());
        }
        let mut out_len = 0usize;
        let schema_result =
            unsafe { ffi::marshal_schema_ffi(buf.as_ptr() as *const _, buf.len(), &mut out_len) };
        assert!(!schema_result.is_null());
        let fields = fields
            .iter()
            .map(|(name, typ, id)| (name.to_string(), *typ, *id))
            .collect();
        EncoderSchema {
            schema_result: ffi::SchemaResultPtr(schema_result),
            fields,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn as_bytes(&self) -> &[u8] {
        unsafe {
            let schema = &*self.schema_result.0;
            std::slice::from_raw_parts(schema.schema_bytes as *const u8, schema.schema_bytes_len)
        }
    }
}

impl EncoderRow {
    #[allow(dead_code)]
    pub(crate) fn from_schema_and_row(schema: &EncoderSchema, row: &[u8]) -> Self {
        let mut out_len = 0usize;
        let ptr = unsafe {
            let schema = &*schema.schema_result.0;
            ffi::marshal_row_ffi(
                schema.schema_ptr,
                row.as_ptr() as *const _,
                row.len(),
                &mut out_len,
            )
        };
        assert!(!ptr.is_null());
        let bytes = unsafe {
            let slice = slice::from_raw_parts(ptr as *const u8, out_len);
            let v = slice.to_vec();
            ffi::free_row_buf_ffi(ptr);
            v
        };
        EncoderRow { bytes }
    }
    #[allow(dead_code)]
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_from_fields() {
        let fields = &[
            ("foo", 16u8, 1u16), // BT_INT32 = 16
            ("bar", 9u8, 2u16),  // BT_STRING = 9
        ];
        let schema = EncoderSchema::from_fields(fields);
        // Should be accessible
        assert!(!schema.as_bytes().is_empty());
    }

    #[test]
    fn test_row_from_schema_and_row() {
        let fields = &[
            ("foo", 16u8, 1u16), // BT_INT32 = 16
            ("bar", 9u8, 2u16),  // BT_STRING = 9
        ];
        let schema = EncoderSchema::from_fields(fields);

        // Compose a row: foo = 42i32; bar = "hello"
        let mut row = Vec::new();
        row.extend_from_slice(&42i32.to_le_bytes()); // foo
        let s = "hello";
        row.extend_from_slice(&(s.len() as u32).to_le_bytes()); // bar string length (u16 LE)
        row.extend_from_slice(s.as_bytes()); // bar string bytes

        let bond_row = EncoderRow::from_schema_and_row(&schema, &row);
        assert!(!bond_row.bytes.is_empty());
        assert!(!bond_row.as_bytes().is_empty());
    }
}
