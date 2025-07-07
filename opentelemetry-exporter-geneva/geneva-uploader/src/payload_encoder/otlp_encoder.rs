use crate::payload_encoder::bond_encoder::{BondDataType, BondEncodedSchema, BondWriter, FieldDef};
use crate::payload_encoder::central_blob::{CentralBlob, CentralEventEntry, CentralSchemaEntry};
use chrono::{TimeZone, Utc};
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::logs::v1::LogRecord;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

type SchemaCache = Arc<RwLock<HashMap<u64, (BondEncodedSchema, [u8; 16], Vec<FieldDef>)>>>;
type BatchKey = (u64, String);
type EncodedRow = (String, u8, Vec<u8>); // (event_name, level, row_buffer)
type BatchValue = (CentralSchemaEntry, Vec<EncodedRow>);
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

const BT_STRING: u8 = BondDataType::BT_STRING as u8;
const BT_FLOAT: u8 = BondDataType::BT_FLOAT as u8;
const BT_DOUBLE: u8 = BondDataType::BT_DOUBLE as u8;
const BT_INT32: u8 = BondDataType::BT_INT32 as u8;
const BT_INT64: u8 = BondDataType::BT_INT64 as u8;
const BT_BOOL: u8 = BondDataType::BT_BOOL as u8;
const BT_WSTRING: u8 = BondDataType::BT_WSTRING as u8;
// TODO - add more types as needed

/// Encoder to write OTLP payload in bond form.
#[derive(Clone)]
pub struct OtlpEncoder {
    // TODO - limit cache size or use LRU eviction, and/or add feature flag for caching
    schema_cache: SchemaCache,
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
            let schema_id = Self::calculate_schema_id(&field_specs);
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
                    row: row_buffer,
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
    fn determine_fields(&self, log: &LogRecord) -> Vec<FieldDef> {
        // Pre-allocate with estimated capacity to avoid reallocations
        let estimated_capacity = 7 + 4 + log.attributes.len();
        let mut fields = Vec::with_capacity(estimated_capacity);
        fields.push((Cow::Borrowed(FIELD_ENV_NAME), BT_STRING));
        fields.push((FIELD_ENV_VER.into(), BT_STRING));
        fields.push((FIELD_TIMESTAMP.into(), BT_STRING));
        fields.push((FIELD_ENV_TIME.into(), BT_STRING));

        // Part A extension - Conditional fields
        if !log.trace_id.is_empty() {
            fields.push((FIELD_TRACE_ID.into(), BT_STRING));
        }
        if !log.span_id.is_empty() {
            fields.push((FIELD_SPAN_ID.into(), BT_STRING));
        }
        if log.flags != 0 {
            fields.push((FIELD_TRACE_FLAGS.into(), BT_INT32));
        }

        // Part B - Core log fields
        if !log.event_name.is_empty() {
            fields.push((FIELD_NAME.into(), BT_STRING));
        }
        fields.push((FIELD_SEVERITY_NUMBER.into(), BT_INT32));
        if !log.severity_text.is_empty() {
            fields.push((FIELD_SEVERITY_TEXT.into(), BT_STRING));
        }
        if let Some(body) = &log.body {
            if let Some(Value::StringValue(_)) = &body.value {
                // Only included in schema when body is a string value
                fields.push((FIELD_BODY.into(), BT_STRING));
            }
            //TODO - handle other body types
        }

        // Part C - Dynamic attributes
        for attr in &log.attributes {
            if let Some(val) = attr.value.as_ref().and_then(|v| v.value.as_ref()) {
                let type_id = match val {
                    Value::StringValue(_) => BondDataType::BT_STRING as u8,
                    Value::IntValue(_) => BondDataType::BT_INT64 as u8,
                    Value::DoubleValue(_) => BondDataType::BT_FLOAT as u8, // TODO - using float for now
                    Value::BoolValue(_) => BondDataType::BT_BOOL as u8, // representing bool as int
                    _ => continue,
                };
                fields.push((attr.key.clone().into(), type_id));
            }
        }
        fields.sort_by(|a, b| a.0.cmp(&b.0)); // Sort fields by name consistent schema ID generation
        fields
            .into_iter()
            .enumerate()
            .map(|(i, (name, type_id))| FieldDef {
                name,
                type_id,
                field_id: (i + 1) as u16,
            })
            .collect()
    }

    /// Calculate schema ID from field specifications
    fn calculate_schema_id(fields: &[FieldDef]) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();

        for field in fields {
            field.name.hash(&mut hasher);
            field.type_id.hash(&mut hasher);
        }

        hasher.finish()
    }

    /// Get or create schema with field ordering information
    fn get_or_create_schema(
        &self,
        schema_id: u64,
        field_info: Vec<FieldDef>,
    ) -> (CentralSchemaEntry, Vec<FieldDef>) {
        // Check cache first
        if let Some((schema, schema_md5, field_info)) =
            self.schema_cache.read().unwrap().get(&schema_id)
        {
            return (
                CentralSchemaEntry {
                    id: schema_id,
                    md5: *schema_md5,
                    schema: schema.clone(),
                },
                field_info.clone(),
            );
        }

        let schema =
            BondEncodedSchema::from_fields("OtlpLogRecord", "telemetry", field_info.clone()); //TODO - use actual struct name and namespace

        let schema_bytes = schema.as_bytes();
        let schema_md5 = md5::compute(schema_bytes).0;
        // Cache the schema and field info
        {
            let mut cache = self.schema_cache.write().unwrap();
            // TODO: Refactor to eliminate field_info duplication in cache
            // The field information (name, type_id, order) is already stored in BondEncodedSchema's
            // DynamicSchema.fields vector. We should:
            // 1. Ensure DynamicSchema maintains fields in sorted order
            // 2. Add a method to BondEncodedSchema to iterate fields for row encoding
            // 3. Remove field_info from cache tuple to reduce memory usage and cloning overhead
            // This would require updating write_row_data() to work with DynamicSchema fields directly
            cache.insert(schema_id, (schema.clone(), schema_md5, field_info.clone()));
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
                    BondWriter::write_int32(&mut buffer, log.flags as i32);
                }
                FIELD_NAME => {
                    BondWriter::write_string(&mut buffer, &log.event_name);
                }
                FIELD_SEVERITY_NUMBER => BondWriter::write_int32(&mut buffer, log.severity_number),
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
        expected_type: u8,
    ) {
        if let Some(val) = &attr.value {
            match (&val.value, expected_type) {
                (Some(Value::StringValue(s)), BT_STRING) => BondWriter::write_string(buffer, s),
                (Some(Value::StringValue(s)), BT_WSTRING) => BondWriter::write_wstring(buffer, s),
                (Some(Value::IntValue(i)), BT_INT64) => BondWriter::write_int64(buffer, *i),
                (Some(Value::DoubleValue(d)), BT_FLOAT) => {
                    // TODO - handle double values properly
                    BondWriter::write_float(buffer, *d as f32)
                }
                (Some(Value::DoubleValue(d)), BT_DOUBLE) => BondWriter::write_double(buffer, *d),
                (Some(Value::BoolValue(b)), BT_BOOL) => {
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
}
