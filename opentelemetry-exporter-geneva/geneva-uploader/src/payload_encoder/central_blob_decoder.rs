#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    const TERMINATOR: u64 = 0xdeadc0dedeadc0de;

    /// A decoded schema from the CentralBlob
    #[derive(Debug, Clone, PartialEq)]
    pub struct DecodedSchema {
        pub id: u64,
        pub md5: [u8; 16],
        pub schema_bytes: Vec<u8>,
    }

    /// A decoded event from the CentralBlob
    #[derive(Debug, Clone, PartialEq)]
    pub struct DecodedEvent {
        pub schema_id: u64,
        pub level: u8,
        pub event_name: String,
        pub row_data: Vec<u8>,
    }

    impl DecodedEvent {
        /// Check if a string value is present in the row data
        /// This is the only method actually used in the tests
        pub fn contains_string_value(&self, value: &str) -> bool {
            let value_bytes = value.as_bytes();

            // Try different string length encodings that Bond might use
            // Bond can use variable-length encoding for strings

            // First try with u32 length prefix (most common)
            let length_bytes = (value_bytes.len() as u32).to_le_bytes();
            if let Some(pos) = self
                .row_data
                .windows(length_bytes.len())
                .position(|window| window == length_bytes)
            {
                let string_start = pos + length_bytes.len();
                if string_start + value_bytes.len() <= self.row_data.len() {
                    if &self.row_data[string_start..string_start + value_bytes.len()] == value_bytes
                    {
                        return true;
                    }
                }
            }

            // Try with u16 length prefix
            if value_bytes.len() <= u16::MAX as usize {
                let length_bytes = (value_bytes.len() as u16).to_le_bytes();
                if let Some(pos) = self
                    .row_data
                    .windows(length_bytes.len())
                    .position(|window| window == length_bytes)
                {
                    let string_start = pos + length_bytes.len();
                    if string_start + value_bytes.len() <= self.row_data.len() {
                        if &self.row_data[string_start..string_start + value_bytes.len()]
                            == value_bytes
                        {
                            return true;
                        }
                    }
                }
            }

            // Try with u8 length prefix for short strings
            if value_bytes.len() <= u8::MAX as usize {
                let length_byte = value_bytes.len() as u8;
                if let Some(pos) = self.row_data.iter().position(|&b| b == length_byte) {
                    let string_start = pos + 1;
                    if string_start + value_bytes.len() <= self.row_data.len() {
                        if &self.row_data[string_start..string_start + value_bytes.len()]
                            == value_bytes
                        {
                            return true;
                        }
                    }
                }
            }

            // As a fallback, just check if the string bytes appear anywhere in the data
            // This is less precise but more likely to catch the value
            self.row_data
                .windows(value_bytes.len())
                .any(|window| window == value_bytes)
        }
    }

    /// The decoded CentralBlob payload
    #[derive(Debug, Clone, PartialEq)]
    pub struct DecodedCentralBlob {
        pub version: u32,
        pub format: u32,
        pub metadata: String,
        pub schemas: Vec<DecodedSchema>,
        pub events: Vec<DecodedEvent>,
    }

    /// Simple CentralBlob decoder for testing purposes
    pub struct CentralBlobDecoder;

    impl CentralBlobDecoder {
        /// Decode a CentralBlob from bytes
        pub fn decode(data: &[u8]) -> Result<DecodedCentralBlob, String> {
            let mut cursor = Cursor::new(data);

            // Read header
            let version = Self::read_u32(&mut cursor)?;
            let format = Self::read_u32(&mut cursor)?;

            // Read metadata
            let metadata_len = Self::read_u32(&mut cursor)?;
            let metadata = Self::read_utf16le_string(&mut cursor, metadata_len as usize)?;

            // Read schemas and events
            let mut schemas = Vec::new();
            let mut events = Vec::new();

            while cursor.position() < data.len() as u64 {
                let entity_type = Self::read_u16(&mut cursor)?;

                match entity_type {
                    0 => {
                        // Schema entry
                        let schema = Self::decode_schema(&mut cursor)?;
                        schemas.push(schema);
                    }
                    2 => {
                        // Event entry
                        let event = Self::decode_event(&mut cursor)?;
                        events.push(event);
                    }
                    _ => return Err(format!("Invalid entity type: {}", entity_type)),
                }
            }

            Ok(DecodedCentralBlob {
                version,
                format,
                metadata,
                schemas,
                events,
            })
        }

        fn decode_schema(cursor: &mut Cursor<&[u8]>) -> Result<DecodedSchema, String> {
            let id = Self::read_u64(cursor)?;
            let mut md5 = [0u8; 16];
            cursor
                .read_exact(&mut md5)
                .map_err(|_| "Unexpected end of data".to_string())?;

            let schema_len = Self::read_u32(cursor)?;
            let mut schema_bytes = vec![0u8; schema_len as usize];
            cursor
                .read_exact(&mut schema_bytes)
                .map_err(|_| "Unexpected end of data".to_string())?;

            let terminator = Self::read_u64(cursor)?;
            if terminator != TERMINATOR {
                return Err("Invalid terminator".to_string());
            }

            Ok(DecodedSchema {
                id,
                md5,
                schema_bytes,
            })
        }

        fn decode_event(cursor: &mut Cursor<&[u8]>) -> Result<DecodedEvent, String> {
            let schema_id = Self::read_u64(cursor)?;
            let level = Self::read_u8(cursor)?;

            let event_name_len = Self::read_u16(cursor)?;
            let event_name = Self::read_utf16le_string(cursor, event_name_len as usize)?;

            let row_len = Self::read_u32(cursor)?;
            let mut row_data = vec![0u8; row_len as usize];
            cursor
                .read_exact(&mut row_data)
                .map_err(|_| "Unexpected end of data".to_string())?;

            let terminator = Self::read_u64(cursor)?;
            if terminator != TERMINATOR {
                return Err("Invalid terminator".to_string());
            }

            Ok(DecodedEvent {
                schema_id,
                level,
                event_name,
                row_data,
            })
        }

        fn read_u8(cursor: &mut Cursor<&[u8]>) -> Result<u8, String> {
            let mut buf = [0u8; 1];
            cursor
                .read_exact(&mut buf)
                .map_err(|_| "Unexpected end of data".to_string())?;
            Ok(buf[0])
        }

        fn read_u16(cursor: &mut Cursor<&[u8]>) -> Result<u16, String> {
            let mut buf = [0u8; 2];
            cursor
                .read_exact(&mut buf)
                .map_err(|_| "Unexpected end of data".to_string())?;
            Ok(u16::from_le_bytes(buf))
        }

        fn read_u32(cursor: &mut Cursor<&[u8]>) -> Result<u32, String> {
            let mut buf = [0u8; 4];
            cursor
                .read_exact(&mut buf)
                .map_err(|_| "Unexpected end of data".to_string())?;
            Ok(u32::from_le_bytes(buf))
        }

        fn read_u64(cursor: &mut Cursor<&[u8]>) -> Result<u64, String> {
            let mut buf = [0u8; 8];
            cursor
                .read_exact(&mut buf)
                .map_err(|_| "Unexpected end of data".to_string())?;
            Ok(u64::from_le_bytes(buf))
        }

        fn read_utf16le_string(
            cursor: &mut Cursor<&[u8]>,
            byte_len: usize,
        ) -> Result<String, String> {
            let mut buf = vec![0u8; byte_len];
            cursor
                .read_exact(&mut buf)
                .map_err(|_| "Unexpected end of data".to_string())?;

            // Convert UTF-16LE bytes to UTF-16 code units
            let mut utf16_chars = Vec::new();
            for chunk in buf.chunks_exact(2) {
                let code_unit = u16::from_le_bytes([chunk[0], chunk[1]]);
                utf16_chars.push(code_unit);
            }

            String::from_utf16(&utf16_chars).map_err(|_| "Invalid UTF-16 data".to_string())
        }
    }
}

// Re-export the test types for use in other test modules
#[cfg(test)]
pub use tests::{CentralBlobDecoder, DecodedCentralBlob};
