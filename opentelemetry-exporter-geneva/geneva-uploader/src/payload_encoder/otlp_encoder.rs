use crate::payload_encoder::bond_encoder::{BondEncodedRow, BondEncodedSchema};
use crate::payload_encoder::central_blob::{CentralBlob, CentralEventEntry, CentralSchemaEntry};
use chrono::{TimeZone, Utc};
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::logs::v1::LogRecord;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

type SchemaCache = Arc<RwLock<HashMap<u64, (BondEncodedSchema, Vec<FieldInfo>)>>>;
type BatchKey = (u64, String);
type EncodedRow = (String, u8, Vec<u8>); // (event_name, level, row_buffer)
type BatchValue = (CentralSchemaEntry, Vec<EncodedRow>);
type LogBatches = HashMap<BatchKey, BatchValue>;

/// Encoder to write OTLP payload in bond form.
pub struct OtlpEncoder {
    // TODO - limit cache size or use LRU eviction, and/or add feature flag for caching
    schema_cache: SchemaCache,
}

#[derive(Clone, Debug)]
struct FieldInfo {
    name: String,
    type_id: u8,
    order_id: u16,
}

impl OtlpEncoder {
    pub fn new() -> Self {
        OtlpEncoder {
            schema_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn encode_log_batch<'a, I>(
        &self,
        logs: I,
        metadata: &str,
    ) -> Vec<(u64, String, Vec<u8>, usize)>
    where
        I: Iterator<Item = &'a opentelemetry_proto::tonic::logs::v1::LogRecord>,
    {
        use std::collections::HashMap;

        let mut batches: LogBatches = HashMap::new();

        for log_record in logs {
            // 1. Get schema
            let field_specs = self.determine_fields(log_record);
            let schema_id = self.calculate_schema_id(&field_specs);
            let (schema_entry, field_info) = self.get_or_create_schema(schema_id, field_specs);

            // 2. Encode row
            let row_buffer = self.write_row_data(log_record, &field_info);
            let event_name = if log_record.event_name.is_empty() {
                "Log".to_string()
            } else {
                log_record.event_name.clone()
            };
            let level = log_record.severity_number as u8;

            // 3. Insert into batches - Key is (schema_id, event_name)
            batches
                .entry((schema_id, event_name.clone()))
                .or_insert_with(|| (schema_entry, Vec::new()))
                .1
                .push((event_name, level, row_buffer));
        }

        // 4. Encode blobs (one per schema AND event_name combination)
        let mut blobs = Vec::new();
        for ((schema_id, batch_event_name), (schema_entry, records)) in batches {
            let events: Vec<CentralEventEntry> = records
                .into_iter()
                .map(|(event_name, level, row_buffer)| CentralEventEntry {
                    schema_id,
                    level,
                    event_name,
                    row: BondEncodedRow::from_schema_and_row(&schema_entry.schema, &row_buffer),
                })
                .collect();
            let events_len = events.len();

            let blob = CentralBlob {
                version: 1,
                format: 2,
                metadata: metadata.to_string(),
                schemas: vec![schema_entry],
                events,
            };
            let bytes = blob.to_bytes();
            blobs.push((schema_id, batch_event_name, bytes, events_len));
        }
        blobs
    }

    /// Determine which fields are present in the LogRecord
    fn determine_fields(&self, log: &LogRecord) -> Vec<(String, u8)> {
        let mut fields = vec![
            ("env_name".to_string(), 9),  // BT_STRING
            ("env_ver".to_string(), 9),   // BT_STRING
            ("timestamp".to_string(), 9), // BT_STRING
            ("env_time".to_string(), 9),  // BT_STRING
        ];

        // Part A extension - Conditional fields
        if !log.trace_id.is_empty() {
            fields.push(("env_dt_traceId".to_string(), 9)); // BT_STRING
        }
        if !log.span_id.is_empty() {
            fields.push(("env_dt_spanId".to_string(), 9)); // BT_STRING
        }
        if log.flags != 0 {
            fields.push(("env_dt_traceFlags".to_string(), 16)); // BT_INT32
        }

        // Part B - Core log fields
        if !log.event_name.is_empty() {
            fields.push(("name".to_string(), 9)); // BT_STRING
        }
        fields.push(("SeverityNumber".to_string(), 16)); // BT_INT32
        if !log.severity_text.is_empty() {
            fields.push(("SeverityText".to_string(), 9)); // BT_STRING
        }
        if let Some(body) = &log.body {
            if let Some(Value::StringValue(_)) = &body.value {
                fields.push(("body".to_string(), 9)); // BT_STRING
            }
        }

        // Part C - Dynamic attributes
        for attr in &log.attributes {
            if let Some(val) = attr.value.as_ref().and_then(|v| v.value.as_ref()) {
                let type_id = match val {
                    Value::StringValue(_) => 9, // BT_STRING
                    Value::IntValue(_) => 16,   // BT_INT32
                    Value::DoubleValue(_) => 7, // BT_FLOAT (using float for now)
                    Value::BoolValue(_) => 16,  // BT_INT32 (representing bool as int)
                    _ => continue,
                };
                fields.push((attr.key.clone(), type_id));
            }
        }

        fields
    }

    /// Calculate schema ID from field specifications
    fn calculate_schema_id(&self, fields: &[(String, u8)]) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();

        // Sort fields by name for consistent schema ID
        let mut sorted_fields = fields.to_vec();
        sorted_fields.sort_by(|a, b| a.0.cmp(&b.0));

        for (name, type_id) in sorted_fields {
            name.hash(&mut hasher);
            type_id.hash(&mut hasher);
        }

        hasher.finish()
    }

    /// Get or create schema with field ordering information
    fn get_or_create_schema(
        &self,
        schema_id: u64,
        field_specs: Vec<(String, u8)>,
    ) -> (CentralSchemaEntry, Vec<FieldInfo>) {
        // Check cache first
        if let Some((schema, field_info)) = self.schema_cache.read().unwrap().get(&schema_id) {
            let schema_bytes = schema.as_bytes();
            let schema_md5 = md5::compute(schema_bytes).0;

            return (
                CentralSchemaEntry {
                    id: schema_id,
                    md5: schema_md5,
                    schema: schema.clone(),
                },
                field_info.clone(),
            );
        }

        // Create new schema
        let mut sorted_specs = field_specs.clone();
        sorted_specs.sort_by(|a, b| a.0.cmp(&b.0));

        let field_defs: Vec<(&str, u8, u16)> = sorted_specs
            .iter()
            .enumerate()
            .map(|(i, (name, type_id))| (name.as_str(), *type_id, (i + 1) as u16))
            .collect();

        let schema = BondEncodedSchema::from_fields(&field_defs, "OtlpLogRecord", "telemetry"); //TODO - use actual struct name and namespace

        // Create field info for ordered writing
        let field_info: Vec<FieldInfo> = field_defs
            .iter()
            .map(|(name, type_id, order_id)| FieldInfo {
                name: name.to_string(),
                type_id: *type_id,
                order_id: *order_id,
            })
            .collect();

        // Cache the schema and field info
        {
            let mut cache = self.schema_cache.write().unwrap();
            cache.insert(schema_id, (schema.clone(), field_info.clone()));
        }

        let schema_bytes = schema.as_bytes();
        let schema_md5 = md5::compute(schema_bytes).0;

        (
            CentralSchemaEntry {
                id: schema_id,
                md5: schema_md5,
                schema,
            },
            field_info,
        )
    }

    /// Write row data directly from LogRecord
    fn write_row_data(&self, log: &LogRecord, field_info: &[FieldInfo]) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(1024);

        // Sort by order_id to write in schema order
        let mut sorted_fields = field_info.to_vec();
        sorted_fields.sort_by_key(|f| f.order_id);

        for field in sorted_fields {
            match field.name.as_str() {
                "env_name" => Self::write_string(&mut buffer, "TestEnv"), // TODO - placeholder for actual env name
                "env_ver" => Self::write_string(&mut buffer, "4.0"), // TODO - placeholder for actual env name
                "timestamp" | "env_time" => {
                    let dt = Self::format_timestamp(log.observed_time_unix_nano);
                    Self::write_string(&mut buffer, &dt);
                }
                "env_dt_traceId" => {
                    if !log.trace_id.is_empty() {
                        let hex = hex::encode(&log.trace_id);
                        Self::write_string(&mut buffer, &hex);
                    }
                }
                "env_dt_spanId" => {
                    if !log.span_id.is_empty() {
                        let hex = hex::encode(&log.span_id);
                        Self::write_string(&mut buffer, &hex);
                    }
                }
                "env_dt_traceFlags" => {
                    if log.flags != 0 {
                        Self::write_int32(&mut buffer, log.flags as i32);
                    }
                }
                "name" => {
                    if !log.event_name.is_empty() {
                        Self::write_string(&mut buffer, &log.event_name);
                    }
                }
                "SeverityNumber" => Self::write_int32(&mut buffer, log.severity_number),
                "SeverityText" => {
                    if !log.severity_text.is_empty() {
                        Self::write_string(&mut buffer, &log.severity_text);
                    }
                }
                "body" => {
                    // TODO - handle all types of body values
                    // For now, we only handle string values
                    if let Some(body) = &log.body {
                        if let Some(Value::StringValue(s)) = &body.value {
                            Self::write_string(&mut buffer, s);
                        }
                    }
                }
                _ => {
                    // Handle dynamic attributes
                    if let Some(attr) = log.attributes.iter().find(|a| a.key == field.name) {
                        self.write_attribute_value(&mut buffer, attr, field.type_id);
                    }
                }
            }
        }

        buffer
    }

    /// Write a string value to buffer
    #[inline]
    fn write_string(buffer: &mut Vec<u8>, s: &str) {
        let bytes = s.as_bytes();
        buffer.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        buffer.extend_from_slice(bytes);
    }

    /// Write an int32 value to buffer
    #[inline]
    fn write_int32(buffer: &mut Vec<u8>, value: i32) {
        buffer.extend_from_slice(&value.to_le_bytes());
    }

    /// Write a float value to buffer
    #[inline]
    fn write_float(buffer: &mut Vec<u8>, value: f32) {
        buffer.extend_from_slice(&value.to_le_bytes());
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
        expected_type: u8,
    ) {
        if let Some(val) = &attr.value {
            match (&val.value, expected_type) {
                (Some(Value::StringValue(s)), 9) => Self::write_string(buffer, s),
                (Some(Value::IntValue(i)), 16) => Self::write_int32(buffer, *i as i32),
                (Some(Value::DoubleValue(d)), 7) => Self::write_float(buffer, *d as f32),
                (Some(Value::BoolValue(b)), 16) => {
                    Self::write_int32(buffer, if *b { 1 } else { 0 })
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
        log2.trace_id = vec![1, 2, 3, 4];
        let _result3 = encoder.encode_log_batch([log2].iter(), metadata);
        assert_eq!(encoder.schema_cache.read().unwrap().len(), 2);
    }
}
