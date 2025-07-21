use crate::payload_encoder::bond_encoder::{BondDataType, BondEncodedSchema, BondWriter, FieldDef};
use crate::payload_encoder::central_blob::{CentralBlob, CentralEventEntry, CentralSchemaEntry};
use chrono::{TimeZone, Utc};
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::logs::v1::LogRecord;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

type SchemaCache = Arc<RwLock<HashMap<u64, (Arc<BondEncodedSchema>, [u8; 16])>>>;
type BatchKey = String; //event_name
type BatchValue = (Vec<CentralSchemaEntry>, Vec<CentralEventEntry>); // (schemas, events)
type LogBatches = HashMap<BatchKey, BatchValue>;

const FIELD_ENV_NAME: &str = "env_name";
const FIELD_ENV_VER: &str = "env_ver";
const FIELD_TIMESTAMP: &str = "timestamp";
const FIELD_ENV_TIME: &str = "env_time";
const FIELD_TRACE_ID: &str = "env_dt_traceId";
const FIELD_SPAN_ID: &str = "env_dt_spanId";
const FIELD_TRACE_FLAGS: &str = "env_dt_traceFlags";
const FIELD_NAME: &str = "name";
const FIELD_SEVERITY_NUMBER: &str = "SeverityNumber";
const FIELD_SEVERITY_TEXT: &str = "SeverityText";
const FIELD_BODY: &str = "body";

/// Encoder to write OTLP payload in bond form.
#[derive(Clone)]
pub(crate) struct OtlpEncoder {
    // TODO - limit cache size or use LRU eviction, and/or add feature flag for caching
    schema_cache: SchemaCache,
}

impl OtlpEncoder {
    pub(crate) fn new() -> Self {
        OtlpEncoder {
            schema_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get the number of cached schemas (for testing/debugging purposes)
    #[allow(dead_code)]
    pub(crate) fn schema_cache_size(&self) -> usize {
        self.schema_cache.read().unwrap().len()
    }

    /// Encode a batch of logs into a vector of (event_name, bytes, events_count)
    pub(crate) fn encode_log_batch<'a, I>(
        &self,
        logs: I,
        metadata: &str,
    ) -> Vec<(String, Vec<u8>, usize)>
    //(event_name, bytes, events_count)
    where
        I: IntoIterator<Item = &'a opentelemetry_proto::tonic::logs::v1::LogRecord>,
    {
        use std::collections::HashMap;

        let mut batches: LogBatches = HashMap::new();

        for log_record in logs {
            // 1. Get schema with optimized single-pass field collection and schema ID calculation
            // TODO - optimize this to use Cow<'static, str> to avoid allocation
            let event_name = if log_record.event_name.is_empty() {
                "Log".to_string()
            } else {
                log_record.event_name.clone()
            };
            let event_name_arc = Arc::new(event_name.clone());
            let (field_info, schema_id) =
                Self::determine_fields_and_schema_id(log_record, &event_name);

            let schema_entry = self.get_or_create_schema(schema_id, field_info.as_slice());

            // 2. Encode row
            let row_buffer = self.write_row_data(log_record, &field_info);
            let level = log_record.severity_number as u8;

            // 3. Create batch key
            let batch_key = event_name;

            // 4. Create or get existing batch entry
            let entry = batches
                .entry(batch_key)
                .or_insert_with(|| (Vec::new(), Vec::new()));

            // 5. Add schema entry if not already present (multiple schemas per event_name batch)
            // TODO: Consider HashMap for schema lookup if typical schema count per batch grows beyond ~10
            //       Currently optimized for 4-5 schemas where linear search is faster than HashMap overhead
            if !entry.0.iter().any(|s| s.id == schema_id) {
                entry.0.push(schema_entry);
            }

            // 6. Create CentralEventEntry directly (optimization: no intermediate EncodedRow)
            let central_event = CentralEventEntry {
                schema_id,
                level,
                event_name: event_name_arc,
                row: row_buffer,
            };
            entry.1.push(central_event);
        }

        // 4. Encode blobs (one per event_name, potentially multiple schemas per blob)
        let mut blobs = Vec::new();
        for (batch_event_name, (schema_entries, events)) in batches {
            let events_len = events.len();

            let blob = CentralBlob {
                version: 1,
                format: 2,
                metadata: metadata.to_string(),
                schemas: schema_entries,
                events,
            };
            let bytes = blob.to_bytes();
            blobs.push((batch_event_name, bytes, events_len));
        }
        blobs
    }

    /// Determine fields and calculate schema ID in a single pass for optimal performance
    fn determine_fields_and_schema_id(log: &LogRecord, event_name: &str) -> (Vec<FieldDef>, u64) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Pre-allocate with estimated capacity to avoid reallocations
        let estimated_capacity = 7 + 4 + log.attributes.len();
        let mut fields = Vec::with_capacity(estimated_capacity);

        // Initialize hasher for schema ID calculation
        let mut hasher = DefaultHasher::new();
        event_name.hash(&mut hasher);

        // Part A - Always present fields
        fields.push((Cow::Borrowed(FIELD_ENV_NAME), BondDataType::BT_STRING));
        fields.push((FIELD_ENV_VER.into(), BondDataType::BT_STRING));
        fields.push((FIELD_TIMESTAMP.into(), BondDataType::BT_STRING));
        fields.push((FIELD_ENV_TIME.into(), BondDataType::BT_STRING));

        // Part A extension - Conditional fields
        if !log.trace_id.is_empty() {
            fields.push((FIELD_TRACE_ID.into(), BondDataType::BT_STRING));
        }
        if !log.span_id.is_empty() {
            fields.push((FIELD_SPAN_ID.into(), BondDataType::BT_STRING));
        }
        if log.flags != 0 {
            fields.push((FIELD_TRACE_FLAGS.into(), BondDataType::BT_INT32));
        }

        // Part B - Core log fields
        if !log.event_name.is_empty() {
            fields.push((FIELD_NAME.into(), BondDataType::BT_STRING));
        }
        fields.push((FIELD_SEVERITY_NUMBER.into(), BondDataType::BT_INT32));
        if !log.severity_text.is_empty() {
            fields.push((FIELD_SEVERITY_TEXT.into(), BondDataType::BT_STRING));
        }
        if let Some(body) = &log.body {
            if let Some(Value::StringValue(_)) = &body.value {
                // Only included in schema when body is a string value
                fields.push((FIELD_BODY.into(), BondDataType::BT_STRING));
            }
            //TODO - handle other body types
        }

        // Part C - Dynamic attributes
        for attr in &log.attributes {
            if let Some(val) = attr.value.as_ref().and_then(|v| v.value.as_ref()) {
                let type_id = match val {
                    Value::StringValue(_) => BondDataType::BT_STRING,
                    Value::IntValue(_) => BondDataType::BT_INT64,
                    Value::DoubleValue(_) => BondDataType::BT_DOUBLE,
                    Value::BoolValue(_) => BondDataType::BT_BOOL,
                    _ => continue,
                };
                fields.push((attr.key.clone().into(), type_id));
            }
        }

        // Sort fields by name for consistent schema ID generation
        fields.sort_by(|a, b| a.0.cmp(&b.0));

        // Hash field names and types while converting to FieldDef
        let field_defs: Vec<FieldDef> = fields
            .into_iter()
            .enumerate()
            .map(|(i, (name, type_id))| {
                // Hash field name and type for schema ID
                name.hash(&mut hasher);
                type_id.hash(&mut hasher);

                FieldDef {
                    name,
                    type_id,
                    field_id: (i + 1) as u16,
                }
            })
            .collect();

        let schema_id = hasher.finish();
        (field_defs, schema_id)
    }

    /// Get or create schema - fields are accessible via returned schema entry
    fn get_or_create_schema(&self, schema_id: u64, field_info: &[FieldDef]) -> CentralSchemaEntry {
        {
            if let Some((schema_arc, schema_md5)) =
                self.schema_cache.read().unwrap().get(&schema_id)
            {
                return CentralSchemaEntry {
                    id: schema_id,
                    md5: *schema_md5,
                    schema: (**schema_arc).clone(), // Dereference Arc and clone BondEncodedSchema
                };
            }
        }

        // Only clone field_info when we actually need to create a new schema
        // Investigate if we can avoid cloning by using Cow using Arc to fields_info
        let schema =
            BondEncodedSchema::from_fields("OtlpLogRecord", "telemetry", field_info.to_vec()); //TODO - use actual struct name and namespace

        let schema_bytes = schema.as_bytes();
        let schema_md5 = md5::compute(schema_bytes).0;

        // Cache Arc<BondEncodedSchema> to avoid cloning large structures
        let schema_arc = Arc::new(schema.clone());
        {
            let mut cache = self.schema_cache.write().unwrap();
            cache.insert(schema_id, (schema_arc, schema_md5));
        }

        CentralSchemaEntry {
            id: schema_id,
            md5: schema_md5,
            schema,
        }
    }

    /// Write row data directly from LogRecord
    fn write_row_data(&self, log: &LogRecord, sorted_fields: &[FieldDef]) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(sorted_fields.len() * 50); //TODO - estimate better

        for field in sorted_fields {
            match field.name.as_ref() {
                FIELD_ENV_NAME => BondWriter::write_string(&mut buffer, "TestEnv"), // TODO - placeholder for actual env name
                FIELD_ENV_VER => BondWriter::write_string(&mut buffer, "4.0"), // TODO - placeholder for actual env version
                FIELD_TIMESTAMP | FIELD_ENV_TIME => {
                    let dt = Self::format_timestamp(log.observed_time_unix_nano);
                    BondWriter::write_string(&mut buffer, &dt);
                }
                FIELD_TRACE_ID => {
                    let hex_bytes = Self::encode_id_to_hex::<32>(&log.trace_id);
                    let hex_str = std::str::from_utf8(&hex_bytes).unwrap();
                    BondWriter::write_string(&mut buffer, hex_str);
                }
                FIELD_SPAN_ID => {
                    let hex_bytes = Self::encode_id_to_hex::<16>(&log.span_id);
                    let hex_str = std::str::from_utf8(&hex_bytes).unwrap();
                    BondWriter::write_string(&mut buffer, hex_str);
                }
                FIELD_TRACE_FLAGS => {
                    BondWriter::write_numeric(&mut buffer, log.flags as i32);
                }
                FIELD_NAME => {
                    BondWriter::write_string(&mut buffer, &log.event_name);
                }
                FIELD_SEVERITY_NUMBER => {
                    BondWriter::write_numeric(&mut buffer, log.severity_number)
                }
                FIELD_SEVERITY_TEXT => {
                    BondWriter::write_string(&mut buffer, &log.severity_text);
                }
                FIELD_BODY => {
                    // TODO - handle all types of body values - For now, we only handle string values
                    if let Some(body) = &log.body {
                        if let Some(Value::StringValue(s)) = &body.value {
                            BondWriter::write_string(&mut buffer, s);
                        }
                    }
                }
                _ => {
                    // Handle dynamic attributes
                    // TODO - optimize better - we could update determine_fields to also return a vec of bytes which has bond serialized attributes
                    if let Some(attr) = log.attributes.iter().find(|a| a.key == field.name) {
                        self.write_attribute_value(&mut buffer, attr, field.type_id);
                    }
                }
            }
        }

        buffer
    }

    fn encode_id_to_hex<const N: usize>(id: &[u8]) -> [u8; N] {
        let mut hex_bytes = [0u8; N];
        hex::encode_to_slice(id, &mut hex_bytes).unwrap();
        hex_bytes
    }

    /// Format timestamp from nanoseconds
    fn format_timestamp(nanos: u64) -> String {
        let secs = (nanos / 1_000_000_000) as i64;
        let nsec = (nanos % 1_000_000_000) as u32;
        Utc.timestamp_opt(secs, nsec)
            .single()
            .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap())
            .to_rfc3339()
    }

    /// Write attribute value based on its type
    fn write_attribute_value(
        &self,
        buffer: &mut Vec<u8>,
        attr: &opentelemetry_proto::tonic::common::v1::KeyValue,
        expected_type: BondDataType,
    ) {
        if let Some(val) = &attr.value {
            match (&val.value, expected_type) {
                (Some(Value::StringValue(s)), BondDataType::BT_STRING) => {
                    BondWriter::write_string(buffer, s)
                }
                (Some(Value::IntValue(i)), BondDataType::BT_INT64) => {
                    BondWriter::write_numeric(buffer, *i)
                }
                (Some(Value::DoubleValue(d)), BondDataType::BT_DOUBLE) => {
                    BondWriter::write_numeric(buffer, *d)
                }
                (Some(Value::BoolValue(b)), BondDataType::BT_BOOL) => {
                    // TODO - represent bool as BT_BOOL
                    BondWriter::write_bool(buffer, *b)
                }
                _ => {} // TODO - handle more types
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload_encoder::central_blob_decoder::{CentralBlobDecoder, DecodedCentralBlob};
    use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};

    const TEST_METADATA: &str = "namespace=testNamespace/eventVersion=Ver1v0";

    /// Helper to create a basic log record with optional customizations
    fn create_log_record(event_name: &str, severity: i32) -> LogRecord {
        LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: event_name.to_string(),
            severity_number: severity,
            severity_text: "INFO".to_string(),
            ..Default::default()
        }
    }

    /// Helper to add attributes to a log record
    fn add_attribute(log: &mut LogRecord, key: &str, value: Value) {
        log.attributes.push(KeyValue {
            key: key.to_string(),
            value: Some(AnyValue { value: Some(value) }),
        });
    }

    /// Helper to add trace context to a log record
    fn add_trace_context(log: &mut LogRecord, trace_id: Vec<u8>, span_id: Vec<u8>, flags: u32) {
        log.trace_id = trace_id;
        log.span_id = span_id;
        log.flags = flags;
    }

    /// Helper to decode and validate basic structure
    fn decode_and_validate_structure(
        result: &[(String, Vec<u8>, usize)],
        expected_batches: usize,
    ) -> Vec<(String, DecodedCentralBlob)> {
        assert_eq!(result.len(), expected_batches);

        result
            .iter()
            .map(|(event_name, blob, event_count)| {
                let decoded =
                    CentralBlobDecoder::decode(blob).expect("Blob should decode successfully");

                // Basic structure validation
                assert_eq!(decoded.version, 1);
                assert_eq!(decoded.format, 2);
                assert_eq!(decoded.metadata, TEST_METADATA);
                assert_eq!(decoded.events.len(), *event_count);
                assert!(!decoded.schemas.is_empty());

                (event_name.clone(), decoded)
            })
            .collect()
    }

    #[test]
    fn test_schema_caching_behavior() {
        let encoder = OtlpEncoder::new();

        // Test 1: Same schema should reuse cache
        let log1 = create_log_record("test_event", 9);
        let log2 = create_log_record("test_event", 10); // Same structure, different values

        encoder.encode_log_batch([log1].iter(), TEST_METADATA);
        assert_eq!(encoder.schema_cache_size(), 1);

        encoder.encode_log_batch([log2].iter(), TEST_METADATA);
        assert_eq!(encoder.schema_cache_size(), 1); // No new schema

        // Test 2: Different schema should create new cache entry
        let mut log3 = create_log_record("test_event", 11);
        log3.trace_id = vec![1; 16]; // Different structure

        encoder.encode_log_batch([log3].iter(), TEST_METADATA);
        assert_eq!(encoder.schema_cache_size(), 2); // New schema added
    }

    #[test]
    fn test_event_name_and_batching() {
        let encoder = OtlpEncoder::new();

        // Test 1: Empty event name defaults to "Log"
        let empty_name_log = create_log_record("", 9);
        let result = encoder.encode_log_batch([empty_name_log].iter(), TEST_METADATA);
        let decoded = decode_and_validate_structure(&result, 1);
        assert_eq!(decoded[0].0, "Log");

        // Test 2: Different event names create separate batches
        let log1 = create_log_record("login", 9);
        let log2 = create_log_record("logout", 10);
        let result = encoder.encode_log_batch([log1, log2].iter(), TEST_METADATA);
        let decoded = decode_and_validate_structure(&result, 2);

        let event_names: Vec<&String> = decoded.iter().map(|(name, _)| name).collect();
        assert!(event_names.contains(&&"login".to_string()));
        assert!(event_names.contains(&&"logout".to_string()));

        // Test 3: Same event name with different schemas batched together
        let log3 = create_log_record("user_action", 9);
        let mut log4 = create_log_record("user_action", 10);
        log4.trace_id = vec![1; 16]; // Different schema

        let result = encoder.encode_log_batch([log3, log4].iter(), TEST_METADATA);
        let decoded = decode_and_validate_structure(&result, 1);

        assert_eq!(decoded[0].0, "user_action");
        assert_eq!(decoded[0].1.events.len(), 2);
        assert_eq!(decoded[0].1.schemas.len(), 2); // Different schemas in same batch
    }

    #[test]
    fn test_comprehensive_field_encoding() {
        let encoder = OtlpEncoder::new();

        // Create log with all possible field types
        let mut comprehensive_log = LogRecord {
            observed_time_unix_nano: 1_700_000_123_456_789_000,
            event_name: "comprehensive_test".to_string(),
            severity_number: 9,
            severity_text: "INFO".to_string(),
            body: Some(AnyValue {
                value: Some(Value::StringValue("Log body content".to_string())),
            }),
            ..Default::default()
        };

        // Add trace context
        add_trace_context(
            &mut comprehensive_log,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            vec![1, 2, 3, 4, 5, 6, 7, 8],
            1,
        );

        // Add all attribute types
        add_attribute(
            &mut comprehensive_log,
            "string_attr",
            Value::StringValue("test_value".to_string()),
        );
        add_attribute(&mut comprehensive_log, "int_attr", Value::IntValue(42));
        add_attribute(
            &mut comprehensive_log,
            "double_attr",
            Value::DoubleValue(3.14159),
        );
        add_attribute(&mut comprehensive_log, "bool_attr", Value::BoolValue(true));

        let result = encoder.encode_log_batch([comprehensive_log].iter(), TEST_METADATA);
        let decoded = decode_and_validate_structure(&result, 1);
        let event = &decoded[0].1.events[0];

        // Validate string values are present in the encoded data
        assert!(
            event.contains_string_value("comprehensive_test"),
            "Should contain event name"
        );
        assert!(
            event.contains_string_value("INFO"),
            "Should contain severity text"
        );
        assert!(
            event.contains_string_value("test_value"),
            "Should contain string attribute"
        );
        assert!(
            event.contains_string_value("TestEnv"),
            "Should contain env_name"
        );
        assert!(event.contains_string_value("4.0"), "Should contain env_ver");
        assert!(
            event.contains_string_value("Log body content"),
            "Should contain body content"
        );
        assert!(
            event.contains_string_value("0102030405060708090a0b0c0d0e0f10"),
            "Should contain trace ID"
        );
        assert!(
            event.contains_string_value("0102030405060708"),
            "Should contain span ID"
        );

        // Validate that the log has the expected event name
        assert_eq!(event.event_name, "comprehensive_test");
        assert_eq!(event.level, 9);
        assert!(!event.row_data.is_empty());
    }

    #[test]
    fn test_field_ordering_consistency() {
        let encoder = OtlpEncoder::new();

        // Test that attribute order doesn't affect schema ID (fields are sorted)
        let mut log1 = create_log_record("ordering_test", 9);
        add_attribute(
            &mut log1,
            "attr_z",
            Value::StringValue("value_z".to_string()),
        );
        add_attribute(
            &mut log1,
            "attr_a",
            Value::StringValue("value_a".to_string()),
        );

        let mut log2 = create_log_record("ordering_test", 10);
        add_attribute(
            &mut log2,
            "attr_a",
            Value::StringValue("value_a".to_string()),
        );
        add_attribute(
            &mut log2,
            "attr_z",
            Value::StringValue("value_z".to_string()),
        );

        let result1 = encoder.encode_log_batch([log1].iter(), TEST_METADATA);
        let result2 = encoder.encode_log_batch([log2].iter(), TEST_METADATA);

        let decoded1 = decode_and_validate_structure(&result1, 1);
        let decoded2 = decode_and_validate_structure(&result2, 1);

        // Should have same schema ID despite different attribute order
        assert_eq!(decoded1[0].1.schemas[0].id, decoded2[0].1.schemas[0].id);
    }

    #[test]
    fn test_multiple_schemas_per_batch() {
        let encoder = OtlpEncoder::new();

        // Create logs with same event name but different schemas
        let base_log = create_log_record("mixed_batch", 5);

        let mut trace_log = create_log_record("mixed_batch", 6);
        add_trace_context(&mut trace_log, vec![1; 16], vec![1; 8], 1);

        let mut attr_log = create_log_record("mixed_batch", 7);
        add_attribute(
            &mut attr_log,
            "custom_attr",
            Value::StringValue("value".to_string()),
        );

        let mut full_log = create_log_record("mixed_batch", 8);
        add_trace_context(&mut full_log, vec![2; 16], vec![2; 8], 2);
        add_attribute(&mut full_log, "another_attr", Value::IntValue(100));

        let result = encoder.encode_log_batch(
            [base_log, trace_log, attr_log, full_log].iter(),
            TEST_METADATA,
        );

        let decoded = decode_and_validate_structure(&result, 1);
        let batch = &decoded[0].1;

        // Verify batch structure
        assert_eq!(batch.events.len(), 4);
        assert_eq!(batch.schemas.len(), 4); // Each log has different schema

        // Verify each event references a valid schema
        for event in &batch.events {
            assert!(batch.schemas.iter().any(|s| s.id == event.schema_id));
            assert_eq!(event.event_name, "mixed_batch");
        }

        // Verify schema uniqueness
        let mut schema_ids: Vec<u64> = batch.schemas.iter().map(|s| s.id).collect();
        schema_ids.sort();
        schema_ids.dedup();
        assert_eq!(schema_ids.len(), batch.schemas.len());
    }

    #[test]
    fn test_minimal_vs_maximal_logs() {
        let encoder = OtlpEncoder::new();

        // Minimal log (only required fields)
        let minimal = create_log_record("minimal", 5);

        // Maximal log (all possible fields)
        let mut maximal = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: "maximal".to_string(),
            severity_number: 12,
            severity_text: "ERROR".to_string(),
            trace_id: vec![1; 16],
            span_id: vec![1; 8],
            flags: 3,
            body: Some(AnyValue {
                value: Some(Value::StringValue("Error message".to_string())),
            }),
            ..Default::default()
        };

        // Add multiple attributes of different types
        for (key, value) in [
            ("str", Value::StringValue("string".to_string())),
            ("num", Value::IntValue(999)),
            ("float", Value::DoubleValue(99.9)),
            ("flag", Value::BoolValue(false)),
        ] {
            add_attribute(&mut maximal, key, value);
        }

        let result = encoder.encode_log_batch([minimal, maximal].iter(), TEST_METADATA);
        let decoded = decode_and_validate_structure(&result, 2);

        // Find each batch
        let minimal_batch = decoded.iter().find(|(name, _)| name == "minimal").unwrap();
        let maximal_batch = decoded.iter().find(|(name, _)| name == "maximal").unwrap();

        // Verify minimal log has basic required fields
        let minimal_event = &minimal_batch.1.events[0];
        assert!(
            minimal_event.contains_string_value("TestEnv"),
            "Should contain env_name"
        );
        assert!(
            minimal_event.contains_string_value("4.0"),
            "Should contain env_ver"
        );
        assert!(
            minimal_event.contains_string_value("minimal"),
            "Should contain event name"
        );
        assert!(
            minimal_event.contains_string_value("INFO"),
            "Should contain severity text"
        );

        // Verify maximal log has all fields
        let maximal_event = &maximal_batch.1.events[0];
        assert!(
            maximal_event.contains_string_value("TestEnv"),
            "Should contain env_name"
        );
        assert!(
            maximal_event.contains_string_value("4.0"),
            "Should contain env_ver"
        );
        assert!(
            maximal_event.contains_string_value("maximal"),
            "Should contain event name"
        );
        assert!(
            maximal_event.contains_string_value("ERROR"),
            "Should contain severity text"
        );
        assert!(
            maximal_event.contains_string_value("Error message"),
            "Should contain body"
        );
        assert!(
            maximal_event.contains_string_value("string"),
            "Should contain string attribute"
        );
        // Contains trace context - check for hex patterns that should be present
        // The trace ID should be present in some form in the encoded data
        assert!(
            maximal_event.contains_string_value("0101010101010101"),
            "Should contain part of trace/span ID"
        );

        // Schema should be different
        assert_ne!(minimal_batch.1.schemas[0].id, maximal_batch.1.schemas[0].id);
    }

    #[test]
    fn test_timestamp_and_id_encoding() {
        let encoder = OtlpEncoder::new();

        let mut log = LogRecord {
            observed_time_unix_nano: 1_234_567_890_123_456_789, // Specific timestamp
            event_name: "timestamp_test".to_string(),
            severity_number: 6,
            trace_id: vec![
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66,
                0x77, 0x88,
            ],
            span_id: vec![0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0x11],
            ..Default::default()
        };

        let result = encoder.encode_log_batch([log].iter(), TEST_METADATA);
        let decoded = decode_and_validate_structure(&result, 1);
        let event = &decoded[0].1.events[0];

        // Validate hex encoding of IDs are present in the encoded data
        assert!(
            event.contains_string_value("123456789abcdef01122334455667788"),
            "Should contain trace ID"
        );
        assert!(
            event.contains_string_value("aabbccddeeff0011"),
            "Should contain span ID"
        );

        // Validate timestamp is properly formatted (contains the expected date)
        assert!(
            event.contains_string_value("2009-02-13"),
            "Should contain formatted date from timestamp"
        );

        // Validate basic structure
        assert_eq!(event.event_name, "timestamp_test");
        assert_eq!(event.level, 6);
        assert!(!event.row_data.is_empty());
    }
}
