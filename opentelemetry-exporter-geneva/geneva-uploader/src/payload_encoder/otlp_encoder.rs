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
    use crate::payload_encoder::central_blob_decoder::CentralBlobDecoder;
    use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};

    /// Test basic encoding functionality and schema caching
    #[test]
    fn test_basic_encoding_and_schema_caching() {
        let encoder = OtlpEncoder::new();
        let metadata = "namespace=testNamespace/eventVersion=Ver1v0";

        // Test 1: Basic encoding with attributes
        let mut log = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: "test_event".to_string(),
            severity_number: 9,
            severity_text: "INFO".to_string(),
            ..Default::default()
        };

        log.attributes.push(KeyValue {
            key: "user_id".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("user123".to_string())),
            }),
        });

        log.attributes.push(KeyValue {
            key: "request_count".to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(42)),
            }),
        });

        let result = encoder.encode_log_batch([log].iter(), metadata);
        assert!(!result.is_empty());
        assert_eq!(result[0].0, "test_event");
        assert_eq!(result[0].2, 1);

        // Test 2: Schema caching with same schema
        let log1 = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            severity_number: 9,
            ..Default::default()
        };

        let log2 = LogRecord {
            observed_time_unix_nano: 1_700_000_001_000_000_000,
            severity_number: 10,
            ..Default::default()
        };

        let _result1 = encoder.encode_log_batch([log1].iter(), metadata);
        assert_eq!(encoder.schema_cache_size(), 2); // Previous test + this one

        let _result2 = encoder.encode_log_batch([log2].iter(), metadata);
        assert_eq!(encoder.schema_cache_size(), 2); // Same schema, so no new entry

        // Test 3: Different schema creates new cache entry
        let mut log3 = LogRecord {
            observed_time_unix_nano: 1_700_000_002_000_000_000,
            severity_number: 11,
            ..Default::default()
        };
        log3.trace_id = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

        let _result3 = encoder.encode_log_batch([log3].iter(), metadata);
        assert_eq!(encoder.schema_cache_size(), 3); // New schema with trace_id
    }

    /// Test event name handling and batching behavior
    #[test]
    fn test_event_name_handling_and_batching() {
        let encoder = OtlpEncoder::new();
        let metadata = "namespace=testNamespace/eventVersion=Ver1v0";

        // Test 1: Empty event name defaults to "Log"
        let empty_name_log = LogRecord {
            event_name: "".to_string(),
            severity_number: 9,
            ..Default::default()
        };

        let result = encoder.encode_log_batch([empty_name_log].iter(), metadata);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "Log");
        assert_eq!(result[0].2, 1);

        // Test 2: Different event names create separate batches
        let log1 = LogRecord {
            event_name: "login".to_string(),
            severity_number: 9,
            ..Default::default()
        };

        let log2 = LogRecord {
            event_name: "logout".to_string(),
            severity_number: 10,
            ..Default::default()
        };

        let result = encoder.encode_log_batch([log1, log2].iter(), metadata);
        assert_eq!(result.len(), 2);

        let event_names: Vec<&String> = result.iter().map(|(name, _, _)| name).collect();
        assert!(event_names.contains(&&"login".to_string()));
        assert!(event_names.contains(&&"logout".to_string()));
        assert!(result.iter().all(|(_, _, count)| *count == 1));

        // Test 3: Same event name with different schemas batched together
        let log3 = LogRecord {
            event_name: "user_action".to_string(),
            severity_number: 9,
            ..Default::default()
        };

        let mut log4 = LogRecord {
            event_name: "user_action".to_string(),
            severity_number: 10,
            ..Default::default()
        };
        log4.trace_id = vec![1; 16];

        let mut log5 = LogRecord {
            event_name: "user_action".to_string(),
            severity_number: 11,
            ..Default::default()
        };
        log5.attributes.push(KeyValue {
            key: "user_id".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("user123".to_string())),
            }),
        });

        let result = encoder.encode_log_batch([log3, log4, log5].iter(), metadata);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "user_action");
        assert_eq!(result[0].2, 3);
    }

    /// Test comprehensive field variations and their decoding
    #[test]
    fn test_comprehensive_field_variations_and_decoding() {
        let encoder = OtlpEncoder::new();
        let metadata = "namespace=testNamespace/eventVersion=Ver1v0";

        // Test scenario 1: Minimal log (basic required fields)
        let minimal_log = LogRecord {
            observed_time_unix_nano: 1_600_000_000_000_000_000,
            event_name: "minimal_test".to_string(),
            severity_number: 5,
            severity_text: "DEBUG".to_string(),
            ..Default::default()
        };

        // Test scenario 2: Log with trace context
        let trace_log = LogRecord {
            observed_time_unix_nano: 1_300_000_000_000_000_000,
            event_name: "trace_test".to_string(),
            severity_number: 6,
            severity_text: "INFO".to_string(),
            trace_id: vec![
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66,
                0x77, 0x88,
            ],
            span_id: vec![0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0x11],
            flags: 3,
            ..Default::default()
        };

        // Test scenario 3: Log with various attribute types
        let mut attr_log = LogRecord {
            observed_time_unix_nano: 1_400_000_000_000_000_000,
            event_name: "attr_test".to_string(),
            severity_number: 8,
            severity_text: "WARN".to_string(),
            ..Default::default()
        };

        attr_log.attributes.push(KeyValue {
            key: "service_name".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("test-service".to_string())),
            }),
        });

        attr_log.attributes.push(KeyValue {
            key: "request_id".to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(123456)),
            }),
        });

        attr_log.attributes.push(KeyValue {
            key: "response_time_ms".to_string(),
            value: Some(AnyValue {
                value: Some(Value::DoubleValue(456.789)),
            }),
        });

        attr_log.attributes.push(KeyValue {
            key: "is_success".to_string(),
            value: Some(AnyValue {
                value: Some(Value::BoolValue(false)),
            }),
        });

        // Test scenario 4: Log with string body
        let body_log = LogRecord {
            observed_time_unix_nano: 1_700_000_003_000_000_000,
            event_name: "body_event".to_string(),
            severity_number: 12,
            severity_text: "DEBUG".to_string(),
            body: Some(AnyValue {
                value: Some(Value::StringValue("This is the log body".to_string())),
            }),
            ..Default::default()
        };

        // Test scenario 5: Comprehensive log with all features
        let mut comprehensive_log = LogRecord {
            observed_time_unix_nano: 1_700_000_123_456_789_000,
            event_name: "comprehensive_test".to_string(),
            severity_number: 9,
            severity_text: "INFO".to_string(),
            trace_id: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            span_id: vec![1, 2, 3, 4, 5, 6, 7, 8],
            flags: 1,
            body: Some(AnyValue {
                value: Some(Value::StringValue("Comprehensive log body".to_string())),
            }),
            ..Default::default()
        };

        comprehensive_log.attributes.push(KeyValue {
            key: "bool_attr".to_string(),
            value: Some(AnyValue {
                value: Some(Value::BoolValue(true)),
            }),
        });

        comprehensive_log.attributes.push(KeyValue {
            key: "double_attr".to_string(),
            value: Some(AnyValue {
                value: Some(Value::DoubleValue(3.14159)),
            }),
        });

        comprehensive_log.attributes.push(KeyValue {
            key: "int_attr".to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(42)),
            }),
        });

        comprehensive_log.attributes.push(KeyValue {
            key: "string_attr".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("test_string_value".to_string())),
            }),
        });

        // Encode all logs
        let logs = vec![
            &minimal_log,
            &trace_log,
            &attr_log,
            &body_log,
            &comprehensive_log,
        ];
        let results = encoder.encode_log_batch(logs.iter().copied(), metadata);

        // Verify we get multiple batches due to different event names
        assert_eq!(results.len(), 5);

        // Test decoding for each batch
        for (i, (event_name, encoded_blob, events_count)) in results.iter().enumerate() {
            let decoded = CentralBlobDecoder::decode(encoded_blob)
                .unwrap_or_else(|_| panic!("Failed to decode blob for batch {}", i + 1));

            // Verify basic structure
            assert_eq!(decoded.version, 1);
            assert_eq!(decoded.format, 2);
            assert_eq!(decoded.metadata, metadata);
            assert_eq!(decoded.events.len(), *events_count);
            assert_eq!(decoded.events.len(), 1); // Each batch has one event
            assert!(!decoded.schemas.is_empty());

            let event = &decoded.events[0];
            let schema = &decoded.schemas[0];

            // Verify event properties
            assert_eq!(event.event_name, *event_name);
            assert_eq!(event.schema_id, schema.id);
            assert!(!event.row_data.is_empty());
            assert!(!schema.schema_bytes.is_empty());
        }

        // Verify expected event names are present
        let event_names: Vec<&String> = results.iter().map(|(name, _, _)| name).collect();
        assert!(event_names.contains(&&"minimal_test".to_string()));
        assert!(event_names.contains(&&"trace_test".to_string()));
        assert!(event_names.contains(&&"attr_test".to_string()));
        assert!(event_names.contains(&&"body_event".to_string()));
        assert!(event_names.contains(&&"comprehensive_test".to_string()));
    }

    /// Test multiple logs with same and different schemas
    #[test]
    fn test_multiple_logs_batching_scenarios() {
        let encoder = OtlpEncoder::new();
        let metadata = "namespace=testNamespace/eventVersion=Ver1v0";

        // Test 1: Multiple logs with same schema (same event name and fields)
        let log1 = LogRecord {
            observed_time_unix_nano: 1_200_000_000_000_000_000,
            event_name: "batch_test".to_string(),
            severity_number: 4,
            severity_text: "WARN".to_string(),
            ..Default::default()
        };

        let log2 = LogRecord {
            observed_time_unix_nano: 1_200_000_001_000_000_000,
            event_name: "batch_test".to_string(),
            severity_number: 8,
            severity_text: "ERROR".to_string(),
            ..Default::default()
        };

        let result = encoder.encode_log_batch([log1, log2].iter(), metadata);
        assert_eq!(result.len(), 1); // Batched together

        let (_, encoded_blob, events_count) = &result[0];
        assert_eq!(*events_count, 2);

        let decoded = CentralBlobDecoder::decode(encoded_blob).expect("Failed to decode blob");
        assert_eq!(decoded.schemas.len(), 1); // Same schema
        assert_eq!(decoded.events.len(), 2); // Two events
        assert_eq!(decoded.events[0].level, 4);
        assert_eq!(decoded.events[1].level, 8);

        // Test 2: Multiple logs with different schemas but same event name
        let log3 = LogRecord {
            observed_time_unix_nano: 1_100_000_000_000_000_000,
            event_name: "mixed_schema_test".to_string(),
            severity_number: 5,
            severity_text: "DEBUG".to_string(),
            ..Default::default()
        };

        let mut log4 = LogRecord {
            observed_time_unix_nano: 1_100_000_001_000_000_000,
            event_name: "mixed_schema_test".to_string(),
            severity_number: 6,
            severity_text: "INFO".to_string(),
            ..Default::default()
        };
        log4.trace_id = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

        let result = encoder.encode_log_batch([log3, log4].iter(), metadata);
        assert_eq!(result.len(), 1); // Same event name, batched together

        let (_, encoded_blob, events_count) = &result[0];
        assert_eq!(*events_count, 2);

        let decoded = CentralBlobDecoder::decode(encoded_blob).expect("Failed to decode blob");
        assert_eq!(decoded.schemas.len(), 2); // Different schemas
        assert_eq!(decoded.events.len(), 2); // Two events
        assert_ne!(decoded.schemas[0].id, decoded.schemas[1].id);
        assert_ne!(decoded.events[0].schema_id, decoded.events[1].schema_id);

        // Both events should have same event name
        assert_eq!(decoded.events[0].event_name, "mixed_schema_test");
        assert_eq!(decoded.events[1].event_name, "mixed_schema_test");

        // Verify each event references a valid schema
        let event1_schema_exists = decoded
            .schemas
            .iter()
            .any(|s| s.id == decoded.events[0].schema_id);
        let event2_schema_exists = decoded
            .schemas
            .iter()
            .any(|s| s.id == decoded.events[1].schema_id);
        assert!(event1_schema_exists);
        assert!(event2_schema_exists);
    }

    /// Test field ordering consistency and data consistency
    #[test]
    fn test_field_ordering_and_data_consistency() {
        let encoder = OtlpEncoder::new();
        let metadata = "namespace=testNamespace/eventVersion=Ver1v0";

        // Test 1: Attribute order should not affect schema ID (fields are sorted)
        let mut log1 = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: "test_event".to_string(),
            severity_number: 9,
            severity_text: "INFO".to_string(),
            ..Default::default()
        };

        log1.attributes.push(KeyValue {
            key: "attr_a".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("value_a".to_string())),
            }),
        });
        log1.attributes.push(KeyValue {
            key: "attr_b".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("value_b".to_string())),
            }),
        });

        let mut log2 = LogRecord {
            observed_time_unix_nano: 1_700_000_001_000_000_000,
            event_name: "test_event".to_string(),
            severity_number: 10,
            severity_text: "WARN".to_string(),
            ..Default::default()
        };

        // Same attributes in different order
        log2.attributes.push(KeyValue {
            key: "attr_b".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("value_b".to_string())),
            }),
        });
        log2.attributes.push(KeyValue {
            key: "attr_a".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("value_a".to_string())),
            }),
        });

        let result1 = encoder.encode_log_batch([log1].iter(), metadata);
        let result2 = encoder.encode_log_batch([log2].iter(), metadata);

        // Same schema ID despite different attribute order
        assert_eq!(result1[0].0, result2[0].0);

        let decoded1 = CentralBlobDecoder::decode(&result1[0].1).expect("Failed to decode blob 1");
        let decoded2 = CentralBlobDecoder::decode(&result2[0].1).expect("Failed to decode blob 2");

        assert_eq!(decoded1.schemas[0].id, decoded2.schemas[0].id);

        // Test 2: Data consistency - same input should produce same output
        let log = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: "consistency_test".to_string(),
            severity_number: 9,
            severity_text: "INFO".to_string(),
            trace_id: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            span_id: vec![1, 2, 3, 4, 5, 6, 7, 8],
            flags: 1,
            ..Default::default()
        };

        let result_a = encoder.encode_log_batch([log.clone()].iter(), metadata);
        let result_b = encoder.encode_log_batch([log.clone()].iter(), metadata);

        let decoded_a =
            CentralBlobDecoder::decode(&result_a[0].1).expect("Failed to decode blob A");
        let decoded_b =
            CentralBlobDecoder::decode(&result_b[0].1).expect("Failed to decode blob B");

        // Verify consistency
        assert_eq!(decoded_a.version, decoded_b.version);
        assert_eq!(decoded_a.format, decoded_b.format);
        assert_eq!(decoded_a.metadata, decoded_b.metadata);
        assert_eq!(decoded_a.schemas[0].id, decoded_b.schemas[0].id);
        assert_eq!(decoded_a.schemas[0].md5, decoded_b.schemas[0].md5);
        assert_eq!(decoded_a.events[0].schema_id, decoded_b.events[0].schema_id);
        assert_eq!(decoded_a.events[0].level, decoded_b.events[0].level);
        assert_eq!(
            decoded_a.events[0].event_name,
            decoded_b.events[0].event_name
        );
        assert_eq!(decoded_a.events[0].row_data, decoded_b.events[0].row_data);
    }

    /// Test complex batching scenario with mixed event names and schemas
    #[test]
    fn test_complex_mixed_batching_scenario() {
        let encoder = OtlpEncoder::new();
        let metadata = "namespace=testNamespace/eventVersion=Ver1v0";

        // Create logs with mixed event names and schemas
        let log1 = LogRecord {
            event_name: "user_action".to_string(),
            severity_number: 9,
            ..Default::default()
        };

        let mut log2 = LogRecord {
            event_name: "user_action".to_string(),
            severity_number: 10,
            ..Default::default()
        };
        log2.trace_id = vec![1; 16];

        let log3 = LogRecord {
            event_name: "system_alert".to_string(),
            severity_number: 11,
            ..Default::default()
        };

        let mut log4 = LogRecord {
            event_name: "".to_string(), // Empty event name -> "Log"
            severity_number: 12,
            ..Default::default()
        };
        log4.attributes.push(KeyValue {
            key: "error_code".to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(404)),
            }),
        });

        let result = encoder.encode_log_batch([log1, log2, log3, log4].iter(), metadata);

        // Should create 3 batches: "user_action", "system_alert", "Log"
        assert_eq!(result.len(), 3);

        // Find and verify each batch
        let user_action = result
            .iter()
            .find(|(name, _, _)| name == "user_action")
            .unwrap();
        let system_alert = result
            .iter()
            .find(|(name, _, _)| name == "system_alert")
            .unwrap();
        let log_batch = result.iter().find(|(name, _, _)| name == "Log").unwrap();

        assert_eq!(user_action.2, 2); // 2 events with different schemas
        assert_eq!(system_alert.2, 1); // 1 event
        assert_eq!(log_batch.2, 1); // 1 event

        // Verify user_action batch has multiple schemas
        let user_action_decoded =
            CentralBlobDecoder::decode(&user_action.1).expect("Failed to decode user_action blob");
        assert_eq!(user_action_decoded.events.len(), 2);
        assert_eq!(user_action_decoded.schemas.len(), 2); // Different schemas
        assert_ne!(
            user_action_decoded.events[0].schema_id,
            user_action_decoded.events[1].schema_id
        );

        // Verify all events in user_action batch have correct event name
        for event in &user_action_decoded.events {
            assert_eq!(event.event_name, "user_action");
        }

        // Verify Log batch has correct event name
        let log_decoded =
            CentralBlobDecoder::decode(&log_batch.1).expect("Failed to decode log blob");
        assert_eq!(log_decoded.events[0].event_name, "Log");
    }

    /// Test simple field validation with single record
    #[test]
    fn test_simple_field_validation() {
        let encoder = OtlpEncoder::new();
        let metadata = "namespace=testNamespace/eventVersion=Ver1v0";

        // Create a simple log record
        let mut log_record = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: "test_event".to_string(),
            severity_number: 9,
            severity_text: "INFO".to_string(),
            ..Default::default()
        };

        // Add one attribute for testing
        log_record.attributes.push(KeyValue {
            key: "user_id".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("user123".to_string())),
            }),
        });

        // Encode the log record
        let results = encoder.encode_log_batch([log_record].iter(), metadata);
        assert_eq!(results.len(), 1);

        let (event_name, encoded_blob, events_count) = &results[0];
        assert_eq!(event_name, "test_event");
        assert_eq!(*events_count, 1);

        // Decode the blob
        let decoded = CentralBlobDecoder::decode(encoded_blob).expect("Failed to decode blob");

        // Verify basic structure
        assert_eq!(decoded.events.len(), 1);
        assert_eq!(decoded.schemas.len(), 1);

        let event = &decoded.events[0];
        assert_eq!(event.event_name, "test_event");
        assert_eq!(event.level, 9);
        assert!(!event.row_data.is_empty());

        // Verify the row data contains expected values
        let row_data = &event.row_data;

        // Check for key string values in the encoded data
        assert!(
            contains_string_value(row_data, "user123"),
            "Row data should contain user_id value"
        );
        assert!(
            contains_string_value(row_data, "test_event"),
            "Row data should contain event name"
        );
        assert!(
            contains_string_value(row_data, "INFO"),
            "Row data should contain severity text"
        );
        assert!(
            contains_string_value(row_data, "TestEnv"),
            "Row data should contain env_name"
        );
        assert!(
            contains_string_value(row_data, "4.0"),
            "Row data should contain env_ver"
        );
    }

    /// Helper function to check if a byte sequence contains a string value
    /// This looks for the string length (as u32 little-endian) followed by the string bytes
    fn contains_string_value(data: &[u8], value: &str) -> bool {
        let value_bytes = value.as_bytes();

        // Try different string length encodings that Bond might use
        // Bond can use variable-length encoding for strings

        // First try with u32 length prefix (most common)
        let length_bytes = (value_bytes.len() as u32).to_le_bytes();
        if let Some(pos) = data
            .windows(length_bytes.len())
            .position(|window| window == length_bytes)
        {
            let string_start = pos + length_bytes.len();
            if string_start + value_bytes.len() <= data.len() {
                if &data[string_start..string_start + value_bytes.len()] == value_bytes {
                    return true;
                }
            }
        }

        // Try with u16 length prefix
        if value_bytes.len() <= u16::MAX as usize {
            let length_bytes = (value_bytes.len() as u16).to_le_bytes();
            if let Some(pos) = data
                .windows(length_bytes.len())
                .position(|window| window == length_bytes)
            {
                let string_start = pos + length_bytes.len();
                if string_start + value_bytes.len() <= data.len() {
                    if &data[string_start..string_start + value_bytes.len()] == value_bytes {
                        return true;
                    }
                }
            }
        }

        // Try with u8 length prefix for short strings
        if value_bytes.len() <= u8::MAX as usize {
            let length_byte = value_bytes.len() as u8;
            if let Some(pos) = data.iter().position(|&b| b == length_byte) {
                let string_start = pos + 1;
                if string_start + value_bytes.len() <= data.len() {
                    if &data[string_start..string_start + value_bytes.len()] == value_bytes {
                        return true;
                    }
                }
            }
        }

        // As a fallback, just check if the string bytes appear anywhere in the data
        // This is less precise but more likely to catch the value
        data.windows(value_bytes.len())
            .any(|window| window == value_bytes)
    }
}
