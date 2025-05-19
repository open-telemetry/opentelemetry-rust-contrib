use crate::payload_encoder::central_blob::{CentralBlob, CentralEventEntry, CentralSchemaEntry};
use crate::payload_encoder::{EncoderRow, EncoderSchema};
use smallvec::SmallVec;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Supported value types for the encoder
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum ValueType<'a> {
    Float(f32),
    Int32(i32),
    String(Cow<'a, str>),
    Double(f64),
    WString(Cow<'a, str>),
    // TODO add more types as needed
}

impl<'a> ValueType<'a> {
    /// Get the Serializer type ID for this value
    /// These values map to specific data types: BT_BOOL(2), BT_FLOAT(7), BT_DOUBLE(8),
    /// BT_STRING(9), BT_WSTRING(18), BT_INT32(16), BT_INT64(17)
    #[allow(dead_code)]
    fn value_type_id(&self) -> u8 {
        match self {
            ValueType::Float(_) => 7,    // BT_DOUBLE
            ValueType::Double(_) => 8,   // BT_DOUBLE
            ValueType::Int32(_) => 16,   // BT_INT32
            ValueType::String(_) => 9,   // BT_STRING
            ValueType::WString(_) => 18, // BT_WSTRING
        }
    }

    /// Write the value bytes to a buffer
    #[allow(dead_code)]
    fn write_to_buffer(&self, buffer: &mut Vec<u8>) {
        match self {
            ValueType::Float(v) => {
                let bytes = v.to_le_bytes();
                buffer.extend_from_slice(&bytes);
            }
            ValueType::Int32(v) => {
                let bytes = v.to_le_bytes();
                buffer.extend_from_slice(&bytes)
            }
            ValueType::Double(v) => {
                let bytes = v.to_le_bytes();
                buffer.extend_from_slice(&bytes);
            }
            ValueType::String(v) => {
                let utf8 = v.as_bytes();
                buffer.extend_from_slice(&(utf8.len() as u32).to_le_bytes());
                buffer.extend_from_slice(utf8);
            }
            ValueType::WString(v) => {
                // Convert UTF-8 to UTF-16
                let utf16: Vec<u16> = v.encode_utf16().collect();
                // Write length of UTF-16 string (in code units, not bytes)
                buffer.extend_from_slice(&(utf16.len() as u16).to_le_bytes());
                // Write UTF-16LE bytes
                for code_unit in utf16 {
                    buffer.extend_from_slice(&code_unit.to_le_bytes());
                }
            } // Add more serialization as needed
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct EncoderField<'a> {
    name: Cow<'a, str>,
    value: ValueType<'a>,
}

impl<'a> EncoderField<'a> {
    /// Create a new field with borrowed or owned values
    #[allow(dead_code)]
    pub(crate) fn new<S>(name: S, value: ValueType<'a>) -> Self
    where
        S: Into<Cow<'a, str>>,
    {
        Self {
            name: name.into(),
            value,
        }
    }

    /// Create field with static str and float value
    #[allow(dead_code)]
    pub fn float(name: &'static str, value: f32) -> Self {
        Self {
            name: Cow::Borrowed(name),
            value: ValueType::Float(value),
        }
    }

    /// Create field with static str and int32 value
    #[allow(dead_code)]
    pub fn int32(name: &'static str, value: i32) -> Self {
        Self {
            name: Cow::Borrowed(name),
            value: ValueType::Int32(value),
        }
    }

    /// Create field with static str and borrowed string value
    #[allow(dead_code)]
    pub fn string(name: &'static str, value: &'a str) -> Self {
        Self {
            name: Cow::Borrowed(name),
            value: ValueType::String(Cow::Borrowed(value)),
        }
    }

    /// Create field with static str and owned string value
    #[allow(dead_code)]
    pub fn string_owned(name: &'static str, value: String) -> Self {
        Self {
            name: Cow::Borrowed(name),
            value: ValueType::String(Cow::Owned(value)),
        }
    }

    /// Create field with static str and borrowed wstring value
    #[allow(dead_code)]
    pub fn wstring(name: &'static str, value: &'a str) -> Self {
        Self {
            name: Cow::Borrowed(name),
            value: ValueType::WString(Cow::Borrowed(value)),
        }
    }

    /// Flexible method to create float field with any string type
    #[allow(dead_code)]
    pub fn float_with<S>(name: S, value: f32) -> Self
    where
        S: Into<Cow<'a, str>>,
    {
        Self {
            name: name.into(),
            value: ValueType::Float(value),
        }
    }

    /// Flexible method to create int32 field with any string type
    #[allow(dead_code)]
    pub fn int32_with<S>(name: S, value: i32) -> Self
    where
        S: Into<Cow<'a, str>>,
    {
        Self {
            name: name.into(),
            value: ValueType::Int32(value),
        }
    }

    /// Flexible method to create string field with any string type
    #[allow(dead_code)]
    pub fn string_with<S, T>(name: S, value: T) -> Self
    where
        S: Into<Cow<'a, str>>,
        T: Into<Cow<'a, str>>,
    {
        Self {
            name: name.into(),
            value: ValueType::String(value.into()),
        }
    }

    /// Flexible method to create wstring field with any string type
    #[allow(dead_code)]
    pub fn wstring_with<S, T>(name: S, value: T) -> Self
    where
        S: Into<Cow<'a, str>>,
        T: Into<Cow<'a, str>>,
    {
        Self {
            name: name.into(),
            value: ValueType::WString(value.into()),
        }
    }

    /// Flexible method to create double field with any string type
    #[allow(dead_code)]
    pub fn double_with<S>(name: S, value: f64) -> Self
    where
        S: Into<Cow<'a, str>>,
    {
        Self {
            name: name.into(),
            value: ValueType::Double(value),
        }
    }

    /// Create any field type with flexible name and value types
    #[allow(dead_code)]
    pub fn new_any<S, V>(name: S, value: V) -> Self
    where
        S: Into<Cow<'a, str>>,
        V: Into<ValueType<'a>>,
    {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

// Implement From traits for common value types
impl<'a> From<i32> for ValueType<'a> {
    #[allow(dead_code)]
    fn from(v: i32) -> Self {
        ValueType::Int32(v)
    }
}

impl<'a> From<f32> for ValueType<'a> {
    #[allow(dead_code)]
    fn from(v: f32) -> Self {
        ValueType::Float(v)
    }
}

impl<'a> From<f64> for ValueType<'a> {
    #[allow(dead_code)]
    fn from(v: f64) -> Self {
        ValueType::Double(v)
    }
}

impl<'a> From<&'a str> for ValueType<'a> {
    #[allow(dead_code)]
    fn from(v: &'a str) -> Self {
        ValueType::String(Cow::Borrowed(v))
    }
}

impl<'a> From<String> for ValueType<'a> {
    #[allow(dead_code)]
    fn from(v: String) -> Self {
        ValueType::String(Cow::Owned(v))
    }
}

impl<'a> From<Cow<'a, str>> for ValueType<'a> {
    #[allow(dead_code)]
    fn from(v: Cow<'a, str>) -> Self {
        ValueType::String(v)
    }
}

/// Field ordering information cached per schema
#[derive(Debug)]
struct FieldOrdering {
    ordered_fields: Vec<(String, u16)>, // Field name and order ID
}

/// The main encoder struct
#[allow(dead_code)]
pub struct Encoder {
    schema_cache: Arc<RwLock<HashMap<u64, EncoderSchema>>>,
    ordering_cache: Arc<RwLock<HashMap<u64, FieldOrdering>>>,
}

impl Encoder {
    /// Create a new encoder instance
    #[allow(dead_code)]
    pub fn new() -> Self {
        Encoder {
            schema_cache: Arc::new(RwLock::new(HashMap::new())),
            ordering_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a encoder blob from a simple key-value map
    #[allow(dead_code)]
    pub fn encode<'a>(
        &self,
        fields: &[EncoderField<'a>],
        event_name: &str,
        level: u8,
        metadata: &str,
    ) -> Vec<u8> {
        // 1. Create or retrieve schema from data
        let (schema_id, schema_entry) = self.create_schema_from_fields(fields);

        // 2. Create a row from the fields
        let mut row_buffer = Vec::with_capacity(512);

        // Get cached field ordering
        let field_ordering = {
            let ordering_map = self.ordering_cache.read().unwrap();
            ordering_map.get(&schema_id).unwrap().ordered_fields.clone()
        };
        // Build a map for quick field lookup by name
        let mut field_map = HashMap::with_capacity(fields.len());
        for field in fields {
            field_map.insert(field.name.as_ref(), &field.value);
        }

        // Write values in schema order
        for (field_name, _) in &field_ordering {
            if let Some(value) = field_map.get(field_name.as_str()) {
                value.write_to_buffer(&mut row_buffer);
            }
        }

        let row_obj = EncoderRow::from_schema_and_row(&schema_entry.schema, &row_buffer);

        // 3. Create event entry
        let event = CentralEventEntry {
            schema_id,
            level,
            event_name: event_name.to_string(),
            row: row_obj,
        };

        // 4. Create the central blob
        let blob = CentralBlob {
            version: 1,
            format: 2,
            metadata: metadata.to_string(),
            schemas: vec![schema_entry],
            events: vec![event],
        };

        // 5. Return the serialized blob
        blob.to_bytes()
    }

    /// Create or retrieve a schema for the given data
    #[allow(dead_code)]
    fn create_schema_from_fields<'a>(
        &self,
        fields: &[EncoderField<'a>],
    ) -> (u64, CentralSchemaEntry) {
        // Create a stable order for fields - use SmallVec to avoid allocations for small field sets
        let mut field_defs = SmallVec::<[(&str, u8, u16); 16]>::with_capacity(fields.len());

        // Prepare field definitions for schema creation (sorted by name for deterministic schema)
        for (i, field) in fields.iter().enumerate() {
            field_defs.push((
                field.name.as_ref(),
                field.value.value_type_id(),
                (i + 1) as u16,
            ));
        }

        // Sort by name for deterministic schema creation
        field_defs.sort_by(|a, b| a.0.cmp(b.0));

        // Calculate a hash for the schema
        let schema_id = self.calculate_schema_id(&field_defs);

        // 1. Try schema cache
        if let Some(schema) = self.schema_cache.read().unwrap().get(&schema_id).cloned() {
            let schema_bytes = schema.as_bytes(); // Using as_bytes() which was in the original code
            let schema_md5 = self.md5_bytes(schema_bytes);

            return (
                schema_id,
                CentralSchemaEntry {
                    id: schema_id,
                    md5: schema_md5,
                    schema: schema.clone(),
                },
            );
        }

        // 2. Create new schema and cache it (write lock)
        let schema = EncoderSchema::from_fields(&field_defs);
        {
            self.schema_cache
                .write()
                .unwrap()
                .insert(schema_id, schema.clone());
        }

        // 3. Create and cache field ordering
        let mut ordering = FieldOrdering {
            ordered_fields: Vec::with_capacity(fields.len()),
        };
        for (name, _, id) in &field_defs {
            ordering.ordered_fields.push((name.to_string(), *id));
        }
        ordering.ordered_fields.sort_by_key(|(_, id)| *id);
        {
            self.ordering_cache
                .write()
                .unwrap()
                .entry(schema_id)
                .or_insert(ordering);
        }

        // 4. Create schema entry
        let schema_bytes = schema.as_bytes(); // Using as_bytes() which was in the original code
        let schema_md5 = self.md5_bytes(schema_bytes);

        (
            schema_id,
            CentralSchemaEntry {
                id: schema_id,
                md5: schema_md5,
                schema,
            },
        )
    }

    /// Helper to calculate a schema ID based on the field definitions
    #[allow(dead_code)]
    fn calculate_schema_id(&self, field_defs: &[(&str, u8, u16)]) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        for (name, type_id, field_id) in field_defs {
            name.hash(&mut hasher);
            type_id.hash(&mut hasher);
            field_id.hash(&mut hasher);
        }

        hasher.finish()
    }

    /// Calculate MD5 hash of data
    #[allow(dead_code)]
    fn md5_bytes(&self, data: &[u8]) -> [u8; 16] {
        md5::compute(data).0
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn schema_cache_size(&self) -> usize {
        self.schema_cache.read().unwrap().len()
    }
}

/// Builder for creating Bond events with fluent API
#[allow(dead_code)]
pub struct EncoderEventBuilder<'a> {
    encoder: Arc<Encoder>,
    fields: Vec<EncoderField<'a>>,
}

impl<'a> EncoderEventBuilder<'a> {
    /// Create a new builder
    #[allow(dead_code)]
    pub(crate) fn new(encoder: Arc<Encoder>) -> Self {
        Self {
            encoder,
            fields: Vec::with_capacity(16),
        }
    }

    /// Add a float field
    #[allow(dead_code)]
    pub fn add_float(mut self, name: &'static str, value: f32) -> Self {
        self.fields.push(EncoderField::float(name, value));
        self
    }

    /// Add an int32 field
    #[allow(dead_code)]
    pub fn add_int32(mut self, name: &'static str, value: i32) -> Self {
        self.fields.push(EncoderField::int32(name, value));
        self
    }

    /// Add a string field
    #[allow(dead_code)]
    pub fn add_string(mut self, name: &'static str, value: &'a str) -> Self {
        self.fields.push(EncoderField::string(name, value));
        self
    }

    /// Add a string field from owned String
    #[allow(dead_code)]
    pub fn add_string_owned(mut self, name: &'static str, value: String) -> Self {
        self.fields.push(EncoderField::string_owned(name, value));
        self
    }

    /// Add a wstring field
    #[allow(dead_code)]
    pub fn add_wstring(mut self, name: &'static str, value: &'a str) -> Self {
        self.fields.push(EncoderField::wstring(name, value));
        self
    }

    /// Add a custom field
    #[allow(dead_code)]
    pub fn add_field(mut self, field: EncoderField<'a>) -> Self {
        self.fields.push(field);
        self
    }

    /// Build the event and return the encoded bytes
    #[allow(dead_code)]
    pub fn build(self, event_name: &str, level: u8, metadata: &str) -> Vec<u8> {
        self.encoder
            .encode(&self.fields, event_name, level, metadata)
    }
}

impl Encoder {
    #[allow(dead_code)]
    pub fn builder<'a>(self: &Arc<Self>) -> EncoderEventBuilder<'a> {
        EncoderEventBuilder::new(self.clone())
    }
}

mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    fn test_bond_encoder_with_fields() {
        let encoder = Encoder::new();

        let fields = [
            EncoderField::float("FloatCol", 3.1415),
            EncoderField::int32("IntCol", 42),
            EncoderField::string("StrCol", "hello"),
        ];

        let metadata = "namespace=testNamespace/eventVersion=Ver1v0";
        let payload = encoder.encode(&fields, "test_event", 1, metadata);

        // Basic validation that we got something
        assert!(!payload.is_empty());
    }

    #[test]
    fn test_bond_encoder_with_builder() {
        let encoder = Arc::new(Encoder::new());

        let payload = encoder
            .builder()
            .add_float("FloatCol", 3.1415)
            .add_int32("IntCol", 42)
            .add_string("StrCol", "hello")
            .build(
                "test_event",
                1,
                "namespace=testNamespace/eventVersion=Ver1v0",
            );

        // Basic validation that we got something
        assert!(!payload.is_empty());
    }

    #[test]
    fn test_schema_caching() {
        let encoder = Encoder::new();

        // Create fields
        let fields = [
            EncoderField::float("FloatCol", 3.1415),
            EncoderField::int32("IntCol", 42),
            EncoderField::string("StrCol", "hello"),
        ];

        // First encoding should create and cache the schema
        let metadata = "namespace=testNamespace/eventVersion=Ver1v0";
        let _payload1 = encoder.encode(&fields, "test_event", 1, metadata);

        // Check that we have one schema in the cache
        assert_eq!(encoder.schema_cache_size(), 1);

        // Second encoding with the same fields (different values) should reuse the schema
        let fields2 = [
            EncoderField::float("FloatCol", 2.7182), // Different value
            EncoderField::int32("IntCol", 100),      // Different value
            EncoderField::string("StrCol", "world"), // Different value
        ];

        let _payload2 = encoder.encode(&fields2, "test_event", 1, metadata);

        // Schema cache should still have just one entry
        assert_eq!(encoder.schema_cache_size(), 1);

        // Add a field to create a different schema
        let fields3 = [
            EncoderField::float("FloatCol", 3.1415),
            EncoderField::int32("IntCol", 42),
            EncoderField::string("StrCol", "hello"),
            EncoderField::int32("ExtraField", 99), // New field
        ];

        let _payload3 = encoder.encode(&fields3, "test_event", 1, metadata);

        // Schema cache should now have two entries
        assert_eq!(encoder.schema_cache_size(), 2);

        // Different field order should be considered a different schema
        let fields4 = [
            EncoderField::int32("IntCol", 42),       // Order changed
            EncoderField::string("StrCol", "hello"), // Order changed
            EncoderField::float("FloatCol", 3.1415), // Order changed
        ];

        let _payload4 = encoder.encode(&fields4, "test_event", 1, metadata);

        // Field order doesn't matter for schema ID calculation (it's based on sorted field names)
        // So we should still have just two schemas
        assert_eq!(encoder.schema_cache_size(), 2);

        // Different field types should create a new schema
        let fields5 = [
            EncoderField::float("FloatCol", 3.1415),
            EncoderField::int32("IntCol", 42),
            EncoderField::string("StrCol", "hello"),
            EncoderField::float("ExtraField", 3.14), // Same name as in fields3 but different type
        ];

        let _payload5 = encoder.encode(&fields5, "test_event", 1, metadata);

        // Schema cache should now have three entries
        assert_eq!(encoder.schema_cache_size(), 3);
    }

    #[test]
    fn test_ordering_cache() {
        let encoder = Encoder::new();

        // Create fields with specific order
        let fields = [
            EncoderField::float("FloatCol", 3.1415),
            EncoderField::int32("IntCol", 42),
            EncoderField::string("StrCol", "hello"),
        ];

        // First encoding should create and cache the schema and ordering
        let metadata = "namespace=testNamespace";
        let payload1 = encoder.encode(&fields, "test_event", 1, metadata);

        // Change field values but use same field structure
        let fields2 = [
            EncoderField::float("FloatCol", 99.9),
            EncoderField::int32("IntCol", 123),
            EncoderField::string("StrCol", "world"),
            EncoderField::string("StrCol", "world"),
        ];

        // Re-encode with same structure but different values
        let payload2 = encoder.encode(&fields2, "test_event", 1, metadata);

        // Payloads should be different due to different values
        assert_ne!(payload1, payload2);

        // But schema cache should still have just one entry
        assert_eq!(encoder.schema_cache_size(), 1);

        // Now try with fields in different order
        let fields3 = [
            EncoderField::string("StrCol", "hello"), // Changed order
            EncoderField::int32("IntCol", 42),       // Changed order
            EncoderField::float("FloatCol", 3.1415), // Changed order
        ];

        let payload3 = encoder.encode(&fields3, "test_event", 1, metadata);

        // Since field order is normalized in schema creation,
        // we should still have just one schema
        assert_eq!(encoder.schema_cache_size(), 1);

        // But even more importantly, the payloads should be identical
        // because the field ordering is preserved from the first encoding
        assert_eq!(payload1, payload3);
    }
}
