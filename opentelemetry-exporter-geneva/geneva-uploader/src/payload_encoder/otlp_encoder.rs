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
    use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};

    #[test]
    fn test_encoding() {
        let encoder = OtlpEncoder::new();

        let mut log = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: "test_event".to_string(),
            severity_number: 9,
            severity_text: "INFO".to_string(),
            ..Default::default()
        };

        // Add some attributes
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

        let metadata = "namespace=testNamespace/eventVersion=Ver1v0";
        let result = encoder.encode_log_batch([log].iter(), metadata);

        assert!(!result.is_empty());
    }

    #[test]
    fn test_schema_caching() {
        let encoder = OtlpEncoder::new();

        let log1 = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            severity_number: 9,
            ..Default::default()
        };

        let mut log2 = LogRecord {
            observed_time_unix_nano: 1_700_000_001_000_000_000,
            severity_number: 10,
            ..Default::default()
        };

        let metadata = "namespace=test";

        // First encoding creates schema
        let _result1 = encoder.encode_log_batch([log1].iter(), metadata);
        assert_eq!(encoder.schema_cache.read().unwrap().len(), 1);

        // Second encoding with same schema structure reuses schema
        let _result2 = encoder.encode_log_batch([log2.clone()].iter(), metadata);
        assert_eq!(encoder.schema_cache.read().unwrap().len(), 1);

        // Add trace_id to create different schema
        log2.trace_id = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let _result3 = encoder.encode_log_batch([log2].iter(), metadata);
        assert_eq!(encoder.schema_cache.read().unwrap().len(), 2);
    }

    #[test]
    fn test_single_event_single_schema() {
        let encoder = OtlpEncoder::new();

        let log = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: "test_event".to_string(),
            severity_number: 9,
            ..Default::default()
        };

        let result = encoder.encode_log_batch([log].iter(), "test");

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "test_event");
        assert_eq!(result[0].2, 1); // events_count
    }

    #[test]
    fn test_same_event_name_multiple_schemas() {
        let encoder = OtlpEncoder::new();

        // Schema 1: Basic log
        let log1 = LogRecord {
            event_name: "user_action".to_string(),
            severity_number: 9,
            ..Default::default()
        };

        // Schema 2: With trace_id
        let mut log2 = LogRecord {
            event_name: "user_action".to_string(),
            severity_number: 10,
            ..Default::default()
        };
        log2.trace_id = vec![1; 16];

        // Schema 3: With attributes
        let mut log3 = LogRecord {
            event_name: "user_action".to_string(),
            severity_number: 11,
            ..Default::default()
        };
        log3.attributes.push(KeyValue {
            key: "user_id".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("user123".to_string())),
            }),
        });

        let result = encoder.encode_log_batch([log1, log2, log3].iter(), "test");

        // All should be in one batch with same event_name
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "user_action");
        assert_eq!(result[0].2, 3); // events_count

        // Should have 3 different schemas cached
        assert_eq!(encoder.schema_cache.read().unwrap().len(), 3);
    }

    #[test]
    fn test_different_event_names() {
        let encoder = OtlpEncoder::new();

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

        let result = encoder.encode_log_batch([log1, log2].iter(), "test");

        // Should create 2 separate batches
        assert_eq!(result.len(), 2);

        let event_names: Vec<&String> = result.iter().map(|(name, _, _)| name).collect();
        assert!(event_names.contains(&&"login".to_string()));
        assert!(event_names.contains(&&"logout".to_string()));

        // Each batch should have 1 event
        assert!(result.iter().all(|(_, _, count)| *count == 1));
    }

    #[test]
    fn test_empty_event_name_defaults_to_log() {
        let encoder = OtlpEncoder::new();

        let log = LogRecord {
            event_name: "".to_string(),
            severity_number: 9,
            ..Default::default()
        };

        let result = encoder.encode_log_batch([log].iter(), "test");

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "Log"); // Should default to "Log"
        assert_eq!(result[0].2, 1);
    }

    #[test]
    fn test_mixed_scenario() {
        let encoder = OtlpEncoder::new();

        // event_name1 with schema1
        let log1 = LogRecord {
            event_name: "user_action".to_string(),
            severity_number: 9,
            ..Default::default()
        };

        // event_name1 with schema2 (different schema, same event)
        let mut log2 = LogRecord {
            event_name: "user_action".to_string(),
            severity_number: 10,
            ..Default::default()
        };
        log2.trace_id = vec![1; 16];

        // event_name2 with schema3
        let log3 = LogRecord {
            event_name: "system_alert".to_string(),
            severity_number: 11,
            ..Default::default()
        };

        // empty event_name (defaults to "Log") with schema4
        let mut log4 = LogRecord {
            event_name: "".to_string(),
            severity_number: 12,
            ..Default::default()
        };
        log4.attributes.push(KeyValue {
            key: "error_code".to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(404)),
            }),
        });

        let result = encoder.encode_log_batch([log1, log2, log3, log4].iter(), "test");

        // Should create 3 batches: "user_action", "system_alert", "Log"
        assert_eq!(result.len(), 3);

        // Find each batch and verify counts
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

        // Should have 4 different schemas cached
        assert_eq!(encoder.schema_cache.read().unwrap().len(), 4);
    }

    use crate::payload_encoder::central_blob_decoder::CentralBlobDecoder;

    #[test]
    fn test_comprehensive_encoding_decode_scenarios() {
        let encoder = OtlpEncoder::new();
        let metadata = "namespace=testNamespace/eventVersion=Ver1v0";

        // Test scenario 1: Basic log with minimal fields
        let basic_log = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: "basic_event".to_string(),
            severity_number: 9,
            severity_text: "INFO".to_string(),
            ..Default::default()
        };

        // Test scenario 2: Log with various attribute types
        let mut attributes_log = LogRecord {
            observed_time_unix_nano: 1_700_000_001_000_000_000,
            event_name: "attributes_event".to_string(),
            severity_number: 10,
            severity_text: "WARN".to_string(),
            ..Default::default()
        };

        attributes_log.attributes.push(KeyValue {
            key: "user_id".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("user123".to_string())),
            }),
        });

        attributes_log.attributes.push(KeyValue {
            key: "request_count".to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(42)),
            }),
        });

        attributes_log.attributes.push(KeyValue {
            key: "response_time".to_string(),
            value: Some(AnyValue {
                value: Some(Value::DoubleValue(123.456)),
            }),
        });

        attributes_log.attributes.push(KeyValue {
            key: "success".to_string(),
            value: Some(AnyValue {
                value: Some(Value::BoolValue(true)),
            }),
        });

        // Test scenario 3: Log with trace context
        let trace_log = LogRecord {
            observed_time_unix_nano: 1_700_000_002_000_000_000,
            event_name: "trace_event".to_string(),
            severity_number: 11,
            severity_text: "ERROR".to_string(),
            trace_id: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            span_id: vec![1, 2, 3, 4, 5, 6, 7, 8],
            flags: 1,
            ..Default::default()
        };

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

        // Test scenario 5: Log with empty event name (should default to "Log")
        let empty_name_log = LogRecord {
            observed_time_unix_nano: 1_700_000_004_000_000_000,
            event_name: "".to_string(),
            severity_number: 13,
            severity_text: "FATAL".to_string(),
            ..Default::default()
        };

        // Test scenario 6: Comprehensive log with all possible features
        let mut comprehensive_log = LogRecord {
            observed_time_unix_nano: 1_700_000_005_000_000_000,
            event_name: "comprehensive_event".to_string(),
            severity_number: 14,
            severity_text: "TRACE".to_string(),
            trace_id: vec![16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1],
            span_id: vec![8, 7, 6, 5, 4, 3, 2, 1],
            flags: 2,
            body: Some(AnyValue {
                value: Some(Value::StringValue("Comprehensive log body".to_string())),
            }),
            ..Default::default()
        };

        comprehensive_log.attributes.push(KeyValue {
            key: "service_name".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("test-service".to_string())),
            }),
        });

        comprehensive_log.attributes.push(KeyValue {
            key: "duration_ms".to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(250)),
            }),
        });

        comprehensive_log.attributes.push(KeyValue {
            key: "cpu_usage".to_string(),
            value: Some(AnyValue {
                value: Some(Value::DoubleValue(0.85)),
            }),
        });

        comprehensive_log.attributes.push(KeyValue {
            key: "healthy".to_string(),
            value: Some(AnyValue {
                value: Some(Value::BoolValue(false)),
            }),
        });

        // Encode all logs
        let logs = vec![
            &basic_log,
            &attributes_log,
            &trace_log,
            &body_log,
            &empty_name_log,
            &comprehensive_log,
        ];
        let results = encoder.encode_log_batch(logs.iter().copied(), metadata);

        // Verify we get multiple batches due to different schemas
        assert!(!results.is_empty());
        println!("Total batches generated: {}", results.len());

        // Test each batch by decoding and verifying
        for (i, (event_name, encoded_blob, events_count)) in results.iter().enumerate() {
            println!(
                "Testing batch {}: event_name={}, events_count={}",
                i + 1,
                event_name,
                events_count
            );

            // Decode the blob
            let decoded = CentralBlobDecoder::decode(encoded_blob)
                .unwrap_or_else(|_| panic!("Failed to decode blob for batch {}", i + 1));

            // Verify basic blob structure
            assert_eq!(decoded.version, 1, "Batch {} has incorrect version", i + 1);
            assert_eq!(decoded.format, 2, "Batch {} has incorrect format", i + 1);
            assert_eq!(
                decoded.metadata,
                metadata,
                "Batch {} has incorrect metadata",
                i + 1
            );
            assert!(
                !decoded.schemas.is_empty(),
                "Batch {} should have at least one schema",
                i + 1
            );
            assert_eq!(
                decoded.events.len(),
                *events_count,
                "Batch {} events count mismatch",
                i + 1
            );

            // Verify schema
            let schema = &decoded.schemas[0];
            assert!(
                !schema.schema_bytes.is_empty(),
                "Batch {} schema bytes should not be empty",
                i + 1
            );

            // Verify events
            for (j, event) in decoded.events.iter().enumerate() {
                assert!(
                    !event.row_data.is_empty(),
                    "Batch {} event {} row data should not be empty",
                    i + 1,
                    j + 1
                );

                // Verify event name handling
                if event_name == "Log" {
                    // This should be from the empty_name_log
                    assert_eq!(
                        event.event_name,
                        "Log",
                        "Batch {} event {} should default to 'Log'",
                        i + 1,
                        j + 1
                    );
                } else {
                    assert_eq!(
                        event.event_name,
                        *event_name,
                        "Batch {} event {} name mismatch",
                        i + 1,
                        j + 1
                    );
                }
            }
        }

        // Verify specific scenarios exist in results
        let event_names: Vec<&String> = results.iter().map(|(name, _, _)| name).collect();

        // Check that all expected event names are present
        assert!(
            event_names.contains(&&"basic_event".to_string()),
            "Missing basic_event"
        );
        assert!(
            event_names.contains(&&"attributes_event".to_string()),
            "Missing attributes_event"
        );
        assert!(
            event_names.contains(&&"trace_event".to_string()),
            "Missing trace_event"
        );
        assert!(
            event_names.contains(&&"body_event".to_string()),
            "Missing body_event"
        );
        assert!(
            event_names.contains(&&"Log".to_string()),
            "Missing Log (from empty event name)"
        );
        assert!(
            event_names.contains(&&"comprehensive_event".to_string()),
            "Missing comprehensive_event"
        );

        // Verify schema diversity - different scenarios should produce different schemas
        // Since we can't access schema_id directly from the return value, we'll check for uniqueness by decoding all blobs
        let mut schema_ids = std::collections::HashSet::new();
        for (_, encoded_blob, _) in &results {
            let decoded = CentralBlobDecoder::decode(encoded_blob).expect("Failed to decode blob");
            schema_ids.insert(decoded.schemas[0].id);
        }
        assert!(
            schema_ids.len() >= 4,
            "Should have at least 4 different schemas for different field combinations"
        );

        println!("All decode scenarios passed successfully!");
    }

    #[test]
    fn test_encoding_multiple_logs_same_schema() {
        let encoder = OtlpEncoder::new();

        let log1 = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: "test_event".to_string(),
            severity_number: 9,
            severity_text: "INFO".to_string(),
            ..Default::default()
        };

        let log2 = LogRecord {
            observed_time_unix_nano: 1_700_000_001_000_000_000,
            event_name: "test_event".to_string(),
            severity_number: 10,
            severity_text: "WARN".to_string(),
            ..Default::default()
        };

        let metadata = "namespace=testNamespace/eventVersion=Ver1v0";
        let result = encoder.encode_log_batch([log1, log2].iter(), metadata);

        assert_eq!(result.len(), 1); // Same schema and event name, so should be batched
        let (event_name, encoded_blob, events_count) = &result[0];

        // Decode the blob
        let decoded = CentralBlobDecoder::decode(encoded_blob).expect("Failed to decode blob");

        // Verify the decoded structure
        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.format, 2);
        assert_eq!(decoded.metadata, metadata);
        assert_eq!(decoded.schemas.len(), 1);
        assert_eq!(decoded.events.len(), 2); // Two events in the same batch
        assert_eq!(decoded.events.len(), *events_count);

        // Verify schema
        let schema = &decoded.schemas[0];
        assert!(!schema.schema_bytes.is_empty());

        // Verify events
        for event in &decoded.events {
            assert_eq!(event.event_name, *event_name);
            assert!(!event.row_data.is_empty());
        }

        // Verify different severity levels
        assert_eq!(decoded.events[0].level, 9);
        assert_eq!(decoded.events[1].level, 10);
    }

    #[test]
    fn test_encoding_multiple_logs_different_schemas() {
        let encoder = OtlpEncoder::new();

        let log1 = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: "test_event".to_string(),
            severity_number: 9,
            severity_text: "INFO".to_string(),
            ..Default::default()
        };

        let mut log2 = LogRecord {
            observed_time_unix_nano: 1_700_000_001_000_000_000,
            event_name: "test_event".to_string(),
            severity_number: 10,
            severity_text: "WARN".to_string(),
            ..Default::default()
        };

        // Add trace_id to log2 to create different schema
        log2.trace_id = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

        let metadata = "namespace=testNamespace/eventVersion=Ver1v0";
        let result = encoder.encode_log_batch([log1, log2].iter(), metadata);

        // Because both events have the same event_name, they should be batched together
        // even though they have different schemas
        assert_eq!(result.len(), 1); // Same event name, so should be in same batch

        // Decode the blob
        let decoded = CentralBlobDecoder::decode(&result[0].1).expect("Failed to decode blob");

        // Verify structure - should have multiple schemas in one batch
        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.format, 2);
        assert_eq!(decoded.metadata, metadata);
        assert_eq!(decoded.schemas.len(), 2); // Two different schemas
        assert_eq!(decoded.events.len(), 2); // Two events

        // Verify different schema IDs exist
        assert_ne!(decoded.schemas[0].id, decoded.schemas[1].id);
        assert_ne!(decoded.events[0].schema_id, decoded.events[1].schema_id);
    }

    #[test]
    fn test_encoding_empty_event_name() {
        let encoder = OtlpEncoder::new();

        let log = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: "".to_string(), // Empty event name
            severity_number: 9,
            severity_text: "INFO".to_string(),
            ..Default::default()
        };

        let metadata = "namespace=testNamespace/eventVersion=Ver1v0";
        let result = encoder.encode_log_batch([log].iter(), metadata);

        assert_eq!(result.len(), 1);
        let (event_name, encoded_blob, _) = &result[0];

        // Should default to "Log" when event_name is empty
        assert_eq!(event_name, "Log");

        // Decode the blob
        let decoded = CentralBlobDecoder::decode(encoded_blob).expect("Failed to decode blob");
        assert_eq!(decoded.events[0].event_name, "Log");
    }

    #[test]
    fn test_field_ordering_different_attribute_order() {
        let encoder = OtlpEncoder::new();

        let mut log1 = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: "test_event".to_string(),
            severity_number: 9,
            severity_text: "INFO".to_string(),
            ..Default::default()
        };

        // Add attributes in one order
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

        // Add same attributes in different order
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

        let metadata = "namespace=testNamespace/eventVersion=Ver1v0";
        let result1 = encoder.encode_log_batch([log1].iter(), metadata);
        let result2 = encoder.encode_log_batch([log2].iter(), metadata);

        // Since attributes are sorted by name, different order produces same schema ID
        // This is the expected behavior for consistent schema generation
        assert_eq!(result1[0].0, result2[0].0);

        // Decode both blobs to verify they're still valid
        let decoded1 = CentralBlobDecoder::decode(&result1[0].1).expect("Failed to decode blob 1");
        let decoded2 = CentralBlobDecoder::decode(&result2[0].1).expect("Failed to decode blob 2");

        // Should have same schema ID since attributes are sorted
        assert_eq!(decoded1.schemas[0].id, decoded2.schemas[0].id);

        // Both should have valid structure
        assert_eq!(decoded1.version, 1);
        assert_eq!(decoded2.version, 1);
        assert_eq!(decoded1.events.len(), 1);
        assert_eq!(decoded2.events.len(), 1);
    }

    #[test]
    fn test_field_ordering_consistent_same_order() {
        let encoder = OtlpEncoder::new();

        let mut log1 = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: "test_event".to_string(),
            severity_number: 9,
            severity_text: "INFO".to_string(),
            ..Default::default()
        };

        // Add attributes in specific order
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

        // Add same attributes in same order
        log2.attributes.push(KeyValue {
            key: "attr_a".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("value_a".to_string())),
            }),
        });
        log2.attributes.push(KeyValue {
            key: "attr_b".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("value_b".to_string())),
            }),
        });

        let metadata = "namespace=testNamespace/eventVersion=Ver1v0";
        let result1 = encoder.encode_log_batch([log1].iter(), metadata);
        let result2 = encoder.encode_log_batch([log2].iter(), metadata);

        // Same attribute order should produce same schema ID
        assert_eq!(result1[0].0, result2[0].0);

        // Decode both blobs
        let decoded1 = CentralBlobDecoder::decode(&result1[0].1).expect("Failed to decode blob 1");
        let decoded2 = CentralBlobDecoder::decode(&result2[0].1).expect("Failed to decode blob 2");

        // Should have same schema ID
        assert_eq!(decoded1.schemas[0].id, decoded2.schemas[0].id);
    }

    #[test]
    fn test_multiple_logs_same_event_name_different_schemas_batched_together() {
        let encoder = OtlpEncoder::new();
        let metadata = "namespace=testNamespace/eventVersion=Ver1v0";

        // Create logs with same event_name but different schemas
        // Schema 1: Basic log with minimal fields
        let log1 = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: "user_action".to_string(),
            severity_number: 9,
            severity_text: "INFO".to_string(),
            ..Default::default()
        };

        // Schema 2: Log with trace context (adds trace_id, span_id, flags fields)
        let log2 = LogRecord {
            observed_time_unix_nano: 1_700_000_001_000_000_000,
            event_name: "user_action".to_string(),
            severity_number: 10,
            severity_text: "WARN".to_string(),
            trace_id: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            span_id: vec![1, 2, 3, 4, 5, 6, 7, 8],
            flags: 1,
            ..Default::default()
        };

        // Schema 3: Log with string attributes (adds custom attribute fields)
        let mut log3 = LogRecord {
            observed_time_unix_nano: 1_700_000_002_000_000_000,
            event_name: "user_action".to_string(),
            severity_number: 11,
            severity_text: "ERROR".to_string(),
            ..Default::default()
        };
        log3.attributes.push(KeyValue {
            key: "user_id".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("user123".to_string())),
            }),
        });
        log3.attributes.push(KeyValue {
            key: "session_id".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("sess456".to_string())),
            }),
        });

        // Schema 4: Log with different attribute types (adds numeric and boolean fields)
        let mut log4 = LogRecord {
            observed_time_unix_nano: 1_700_000_003_000_000_000,
            event_name: "user_action".to_string(),
            severity_number: 12,
            severity_text: "DEBUG".to_string(),
            ..Default::default()
        };
        log4.attributes.push(KeyValue {
            key: "request_count".to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(42)),
            }),
        });
        log4.attributes.push(KeyValue {
            key: "response_time".to_string(),
            value: Some(AnyValue {
                value: Some(Value::DoubleValue(123.456)),
            }),
        });
        log4.attributes.push(KeyValue {
            key: "success".to_string(),
            value: Some(AnyValue {
                value: Some(Value::BoolValue(true)),
            }),
        });

        // Schema 5: Log with body field (adds body field)
        let log5 = LogRecord {
            observed_time_unix_nano: 1_700_000_004_000_000_000,
            event_name: "user_action".to_string(),
            severity_number: 13,
            severity_text: "FATAL".to_string(),
            body: Some(AnyValue {
                value: Some(Value::StringValue("Critical error occurred".to_string())),
            }),
            ..Default::default()
        };

        // Schema 6: Log with combination of trace context and attributes
        let mut log6 = LogRecord {
            observed_time_unix_nano: 1_700_000_005_000_000_000,
            event_name: "user_action".to_string(),
            severity_number: 14,
            severity_text: "TRACE".to_string(),
            trace_id: vec![16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1],
            span_id: vec![8, 7, 6, 5, 4, 3, 2, 1],
            flags: 2,
            ..Default::default()
        };
        log6.attributes.push(KeyValue {
            key: "operation".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("login".to_string())),
            }),
        });
        log6.attributes.push(KeyValue {
            key: "duration_ms".to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(250)),
            }),
        });

        // Encode all logs together
        let logs = vec![&log1, &log2, &log3, &log4, &log5, &log6];
        let result = encoder.encode_log_batch(logs.iter().copied(), metadata);

        // Verify all logs are batched together under the same event_name
        assert_eq!(
            result.len(),
            1,
            "All logs should be batched together under same event_name"
        );
        let (event_name, encoded_blob, events_count) = &result[0];
        assert_eq!(event_name, "user_action");
        assert_eq!(*events_count, 6, "Should contain all 6 logs");

        // Decode the blob to verify internal structure
        let decoded = CentralBlobDecoder::decode(encoded_blob).expect("Failed to decode blob");

        // Verify blob structure
        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.format, 2);
        assert_eq!(decoded.metadata, metadata);
        assert_eq!(decoded.events.len(), 6, "Should contain 6 events");

        // Verify multiple schemas are present (since logs have different field combinations)
        assert!(
            decoded.schemas.len() >= 5,
            "Should have at least 5 different schemas due to different field combinations"
        );

        // Verify schema IDs are different for different field combinations
        let schema_ids: std::collections::HashSet<u64> =
            decoded.schemas.iter().map(|s| s.id).collect();
        assert!(
            schema_ids.len() >= 5,
            "Should have at least 5 unique schema IDs"
        );

        // Verify all events have the same event_name
        for (i, event) in decoded.events.iter().enumerate() {
            assert_eq!(
                event.event_name, "user_action",
                "Event {} should have event_name 'user_action'",
                i
            );
            assert!(
                !event.row_data.is_empty(),
                "Event {} should have non-empty row data",
                i
            );
        }

        // Verify each event references a valid schema
        for (i, event) in decoded.events.iter().enumerate() {
            let schema_exists = decoded.schemas.iter().any(|s| s.id == event.schema_id);
            assert!(
                schema_exists,
                "Event {} references a schema that doesn't exist in the blob",
                i
            );
        }

        // Verify different severity levels are preserved
        let severity_levels: Vec<u8> = decoded.events.iter().map(|e| e.level).collect();
        assert_eq!(
            severity_levels,
            vec![9, 10, 11, 12, 13, 14],
            "Severity levels should be preserved"
        );

        // Verify that different schemas are created for different field combinations
        // We can't directly inspect schema fields, but we can verify that logs with different
        // field combinations produce different schema IDs
        let event_schema_ids: Vec<u64> = decoded.events.iter().map(|e| e.schema_id).collect();

        // At minimum, we should have different schema IDs for:
        // - Basic log (log1)
        // - Log with trace context (log2)
        // - Log with string attributes (log3)
        // - Log with different attribute types (log4)
        // - Log with body (log5)
        // - Log with trace + attributes (log6)
        let unique_schema_ids: std::collections::HashSet<u64> =
            event_schema_ids.iter().cloned().collect();
        assert!(
            unique_schema_ids.len() >= 5,
            "Should have at least 5 unique schema IDs for different field combinations"
        );

        println!("Successfully tested batching of {} logs with {} different schemas under event_name '{}'", 
                 decoded.events.len(), decoded.schemas.len(), event_name);
        println!("Schema IDs: {:?}", unique_schema_ids);
    }
}
