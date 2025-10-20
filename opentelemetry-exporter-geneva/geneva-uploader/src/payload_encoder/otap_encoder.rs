// otap_encoder.rs - Direct OTAP Arrow RecordBatch to Bond encoder
//
// This encoder transforms OpenTelemetry Arrow Protocol (OTAP) RecordBatches directly
// to Bond encoding without intermediate OTLP protobuf conversion, eliminating
// unnecessary memory allocations and improving performance.

use crate::client::EncodedBatch;
use crate::payload_encoder::bond_encoder::{BondDataType, BondEncodedSchema, BondWriter, FieldDef};
use crate::payload_encoder::central_blob::{
    BatchMetadata, CentralBlob, CentralEventEntry, CentralSchemaEntry,
};
use crate::payload_encoder::lz4_chunked_compression::lz4_chunked_compression;
use arrow::array::{
    Array, Int32Array, RecordBatch,
    StringArray, TimestampNanosecondArray, UInt32Array,
};
use arrow::datatypes::{DataType, TimeUnit};
use chrono::{TimeZone, Utc};
use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tracing::debug;

// Geneva field name constants (matching OTLP encoder)
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

// OTAP Arrow schema column names
const COL_TIME_UNIX_NANO: &str = "time_unix_nano";
const COL_OBSERVED_TIME_UNIX_NANO: &str = "observed_time_unix_nano";
const COL_TRACE_ID: &str = "trace_id";
const COL_SPAN_ID: &str = "span_id";
const COL_FLAGS: &str = "flags";
const COL_SEVERITY_NUMBER: &str = "severity_number";
const COL_SEVERITY_TEXT: &str = "severity_text";
const COL_BODY: &str = "body";
const COL_EVENT_NAME: &str = "event_name";

/// Encoder to write OTAP Arrow RecordBatch payload in Bond form.
///
/// This encoder directly processes Arrow columnar data without conversion to OTLP,
/// resulting in significant performance improvements through:
/// - Zero-copy columnar access
/// - SIMD-friendly data layout
/// - Reduced memory allocations
#[derive(Clone)]
pub struct OtapEncoder {
    metadata: String,
}

impl OtapEncoder {
    /// Create a new OTAP encoder with Geneva metadata
    pub fn new(metadata: String) -> Self {
        OtapEncoder { metadata }
    }

    /// Encode a batch of logs from Arrow RecordBatch into compressed Bond format.
    ///
    /// The logs are grouped by event_name, and each group gets its own schema and batch.
    /// Returns a vector of EncodedBatch, where each batch contains:
    /// - event_name: Routing identifier
    /// - data: LZ4 chunked compressed Bond-encoded bytes
    /// - metadata: Timestamp range and schema IDs
    ///
    /// # Arguments
    /// * `logs_batch` - Arrow RecordBatch containing log records
    ///
    /// # Returns
    /// * `Ok(Vec<EncodedBatch>)` - Encoded and compressed batches grouped by event_name
    /// * `Err(String)` - Error message if encoding or compression fails
    pub fn encode_log_batch(&self, logs_batch: &RecordBatch) -> Result<Vec<EncodedBatch>, String> {
        // Internal struct to accumulate batch data before encoding
        struct BatchData {
            schemas: Vec<CentralSchemaEntry>,
            events: Vec<CentralEventEntry>,
            metadata: BatchMetadata,
        }

        impl BatchData {
            fn format_schema_ids(&self) -> String {
                use std::fmt::Write;

                if self.schemas.is_empty() {
                    return String::new();
                }

                let estimated_capacity =
                    self.schemas.len() * 32 + self.schemas.len().saturating_sub(1);

                self.schemas.iter().enumerate().fold(
                    String::with_capacity(estimated_capacity),
                    |mut acc, (i, s)| {
                        if i > 0 {
                            acc.push(';');
                        }
                        let md5_hash = md5::compute(s.id.to_le_bytes());
                        write!(&mut acc, "{md5_hash:x}").unwrap();
                        acc
                    },
                )
            }
        }

        let num_rows = logs_batch.num_rows();
        if num_rows == 0 {
            return Ok(Vec::new());
        }

        // Extract Arrow arrays from RecordBatch
        let time_unix_nano = Self::get_timestamp_array(logs_batch, COL_TIME_UNIX_NANO)?;
        let observed_time_unix_nano =
            Self::get_timestamp_array(logs_batch, COL_OBSERVED_TIME_UNIX_NANO)?;
        let trace_id = Self::get_binary_array(logs_batch, COL_TRACE_ID);
        let span_id = Self::get_binary_array(logs_batch, COL_SPAN_ID);
        let flags = Self::get_u32_array(logs_batch, COL_FLAGS);
        let severity_number = Self::get_i32_array(logs_batch, COL_SEVERITY_NUMBER)?;
        let severity_text = Self::get_string_array(logs_batch, COL_SEVERITY_TEXT);
        let body = Self::get_body_string(logs_batch, COL_BODY);
        let event_name_array = Self::get_string_array(logs_batch, COL_EVENT_NAME);

        let mut batches: HashMap<String, BatchData> = HashMap::new();

        // Iterate through each row in the Arrow batch
        for row_idx in 0..num_rows {
            // Get timestamp (prefer time_unix_nano, fall back to observed_time_unix_nano)
            let timestamp = if let Some(arr) = &time_unix_nano {
                if arr.is_valid(row_idx) {
                    arr.value(row_idx) as u64
                } else {
                    0
                }
            } else {
                0
            };

            let timestamp = if timestamp == 0 {
                if let Some(arr) = &observed_time_unix_nano {
                    if arr.is_valid(row_idx) {
                        arr.value(row_idx) as u64
                    } else {
                        0
                    }
                } else {
                    0
                }
            } else {
                timestamp
            };

            // Get event_name (default to "Log" if empty)
            let event_name_str = if let Some(arr) = &event_name_array {
                if arr.is_valid(row_idx) && !arr.value(row_idx).is_empty() {
                    arr.value(row_idx).to_string()
                } else {
                    "Log".to_string()
                }
            } else {
                "Log".to_string()
            };

            // Determine fields and schema ID for this row
            let (field_defs, schema_id) = self.determine_fields_and_schema_id(
                logs_batch,
                row_idx,
                &event_name_str,
                trace_id.as_ref(),
                span_id.as_ref(),
                flags.as_ref(),
                severity_text.as_ref(),
                body.as_ref(),
            )?;

            // Encode row data
            let row_buffer = self.write_row_data(
                logs_batch,
                row_idx,
                &field_defs,
                timestamp,
                trace_id.as_ref(),
                span_id.as_ref(),
                flags.as_ref(),
                &severity_number,
                severity_text.as_ref(),
                body.as_ref(),
                event_name_array.as_ref(),
            )?;

            // Get severity level (defaults to 0 if null)
            let level = if severity_number.is_valid(row_idx) {
                severity_number.value(row_idx) as u8
            } else {
                0
            };

            // Create or get existing batch entry
            let entry = batches.entry(event_name_str.clone()).or_insert_with(|| {
                BatchData {
                    schemas: Vec::new(),
                    events: Vec::new(),
                    metadata: BatchMetadata {
                        start_time: timestamp,
                        end_time: timestamp,
                        schema_ids: String::new(),
                    },
                }
            });

            // Update timestamp range
            if timestamp != 0 {
                entry.metadata.start_time = entry.metadata.start_time.min(timestamp);
                entry.metadata.end_time = entry.metadata.end_time.max(timestamp);
            }

            // Add schema entry if not already present
            if !entry.schemas.iter().any(|s| s.id == schema_id) {
                let schema_entry = Self::create_schema(schema_id, field_defs);
                entry.schemas.push(schema_entry);
            }

            // Create CentralEventEntry
            let central_event = CentralEventEntry {
                schema_id,
                level,
                event_name: Arc::new(event_name_str),
                row: row_buffer,
            };
            entry.events.push(central_event);
        }

        // Encode and compress blobs
        let mut blobs = Vec::with_capacity(batches.len());
        for (batch_event_name, mut batch_data) in batches {
            let schema_ids_string = batch_data.format_schema_ids();
            batch_data.metadata.schema_ids = schema_ids_string;

            let schemas_count = batch_data.schemas.len();
            let events_count = batch_data.events.len();

            let blob = CentralBlob {
                version: 1,
                format: 2,
                metadata: self.metadata.clone(),
                schemas: batch_data.schemas,
                events: batch_data.events,
            };

            let uncompressed = blob.to_bytes();
            let compressed = lz4_chunked_compression(&uncompressed).map_err(|e| {
                debug!(
                    name: "encoder.encode_log_batch.compress_error",
                    target: "geneva-uploader",
                    event_name = %batch_event_name,
                    error = %e,
                    "LZ4 compression failed"
                );
                format!("compression failed: {e}")
            })?;

            debug!(
                name: "encoder.encode_log_batch",
                target: "geneva-uploader",
                event_name = %batch_event_name,
                schemas = schemas_count,
                events = events_count,
                uncompressed_size = uncompressed.len(),
                compressed_size = compressed.len(),
                "Encoded OTAP log batch"
            );

            blobs.push(EncodedBatch {
                event_name: batch_event_name,
                data: compressed,
                metadata: batch_data.metadata,
            });
        }

        Ok(blobs)
    }

    /// Determine fields and calculate schema ID based on Arrow RecordBatch row
    fn determine_fields_and_schema_id(
        &self,
        _batch: &RecordBatch,
        row_idx: usize,
        event_name: &str,
        trace_id: Option<&StringArray>,
        span_id: Option<&StringArray>,
        flags: Option<&UInt32Array>,
        severity_text: Option<&StringArray>,
        body: Option<&StringArray>,
    ) -> Result<(Vec<FieldDef>, u64), String> {
        let mut fields = Vec::with_capacity(15); // Estimated capacity
        let mut hasher = DefaultHasher::new();
        event_name.hash(&mut hasher);

        // Part A - Always present fields
        fields.push((Cow::Borrowed(FIELD_ENV_NAME), BondDataType::BT_STRING));
        fields.push((FIELD_ENV_VER.into(), BondDataType::BT_STRING));
        fields.push((FIELD_TIMESTAMP.into(), BondDataType::BT_STRING));
        fields.push((FIELD_ENV_TIME.into(), BondDataType::BT_STRING));

        // Part A extension - Conditional fields
        if let Some(arr) = trace_id {
            if arr.is_valid(row_idx) && !arr.value(row_idx).is_empty() {
                fields.push((FIELD_TRACE_ID.into(), BondDataType::BT_STRING));
            }
        }
        if let Some(arr) = span_id {
            if arr.is_valid(row_idx) && !arr.value(row_idx).is_empty() {
                fields.push((FIELD_SPAN_ID.into(), BondDataType::BT_STRING));
            }
        }
        if let Some(arr) = flags {
            if arr.is_valid(row_idx) && arr.value(row_idx) != 0 {
                fields.push((FIELD_TRACE_FLAGS.into(), BondDataType::BT_UINT32));
            }
        }

        // Part B - Core log fields
        if !event_name.is_empty() {
            fields.push((FIELD_NAME.into(), BondDataType::BT_STRING));
        }
        fields.push((FIELD_SEVERITY_NUMBER.into(), BondDataType::BT_INT32));
        if let Some(arr) = severity_text {
            if arr.is_valid(row_idx) && !arr.value(row_idx).is_empty() {
                fields.push((FIELD_SEVERITY_TEXT.into(), BondDataType::BT_STRING));
            }
        }
        if let Some(arr) = body {
            if arr.is_valid(row_idx) && !arr.value(row_idx).is_empty() {
                fields.push((FIELD_BODY.into(), BondDataType::BT_STRING));
            }
        }

        // TODO: Add support for dynamic attributes from Arrow schema

        // Convert to FieldDef and hash for schema ID
        let field_defs: Vec<FieldDef> = fields
            .into_iter()
            .enumerate()
            .map(|(i, (name, type_id))| {
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
        Ok((field_defs, schema_id))
    }

    /// Write row data to Bond Simple Binary format
    #[allow(clippy::too_many_arguments)]
    fn write_row_data(
        &self,
        _batch: &RecordBatch,
        row_idx: usize,
        fields: &[FieldDef],
        timestamp: u64,
        trace_id: Option<&StringArray>,
        span_id: Option<&StringArray>,
        flags: Option<&UInt32Array>,
        severity_number: &Int32Array,
        severity_text: Option<&StringArray>,
        body: Option<&StringArray>,
        event_name_array: Option<&StringArray>,
    ) -> Result<Vec<u8>, String> {
        let mut buffer = Vec::with_capacity(fields.len() * 50);

        // Format timestamp once for reuse
        let formatted_timestamp = if timestamp != 0 {
            let secs = (timestamp / 1_000_000_000) as i64;
            let nanos = (timestamp % 1_000_000_000) as u32;
            Utc.timestamp_opt(secs, nanos)
                .single()
                .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap())
                .to_rfc3339()
        } else {
            String::new()
        };

        for field in fields {
            match field.name.as_ref() {
                FIELD_ENV_NAME => {
                    BondWriter::write_string(&mut buffer, "Log");
                }
                FIELD_ENV_VER => {
                    BondWriter::write_string(&mut buffer, "4.0");
                }
                FIELD_TIMESTAMP => {
                    BondWriter::write_string(&mut buffer, &formatted_timestamp);
                }
                FIELD_ENV_TIME => {
                    BondWriter::write_string(&mut buffer, &formatted_timestamp);
                }
                FIELD_TRACE_ID => {
                    if let Some(arr) = trace_id {
                        if arr.is_valid(row_idx) {
                            let value = arr.value(row_idx);
                            let hex = hex::encode(value);
                            BondWriter::write_string(&mut buffer, &hex);
                        }
                    }
                }
                FIELD_SPAN_ID => {
                    if let Some(arr) = span_id {
                        if arr.is_valid(row_idx) {
                            let value = arr.value(row_idx);
                            let hex = hex::encode(value);
                            BondWriter::write_string(&mut buffer, &hex);
                        }
                    }
                }
                FIELD_TRACE_FLAGS => {
                    if let Some(arr) = flags {
                        if arr.is_valid(row_idx) {
                            BondWriter::write_numeric(&mut buffer, arr.value(row_idx));
                        }
                    }
                }
                FIELD_NAME => {
                    if let Some(arr) = event_name_array {
                        if arr.is_valid(row_idx) {
                            BondWriter::write_string(&mut buffer, arr.value(row_idx));
                        }
                    }
                }
                FIELD_SEVERITY_NUMBER => {
                    if severity_number.is_valid(row_idx) {
                        BondWriter::write_numeric(&mut buffer, severity_number.value(row_idx));
                    } else {
                        BondWriter::write_numeric(&mut buffer, 0i32);
                    }
                }
                FIELD_SEVERITY_TEXT => {
                    if let Some(arr) = severity_text {
                        if arr.is_valid(row_idx) {
                            BondWriter::write_string(&mut buffer, arr.value(row_idx));
                        }
                    }
                }
                FIELD_BODY => {
                    if let Some(arr) = body {
                        if arr.is_valid(row_idx) {
                            BondWriter::write_string(&mut buffer, arr.value(row_idx));
                        }
                    }
                }
                _ => {
                    // TODO: Handle dynamic attributes
                }
            }
        }

        Ok(buffer)
    }

    /// Create schema entry from field definitions
    fn create_schema(schema_id: u64, fields: Vec<FieldDef>) -> CentralSchemaEntry {
        let schema_obj = BondEncodedSchema::from_fields("MdsContainer", "Log", fields);
        let schema_bytes = schema_obj.as_bytes();
        let schema_md5 = md5::compute(schema_bytes).0;

        CentralSchemaEntry {
            id: schema_id,
            md5: schema_md5,
            schema: schema_obj,
        }
    }

    // Helper methods to extract typed arrays from RecordBatch

    fn get_timestamp_array(
        batch: &RecordBatch,
        name: &str,
    ) -> Result<Option<TimestampNanosecondArray>, String> {
        match batch.column_by_name(name) {
            Some(col) => match col.data_type() {
                DataType::Timestamp(TimeUnit::Nanosecond, _) => {
                    let arr = col
                        .as_any()
                        .downcast_ref::<TimestampNanosecondArray>()
                        .ok_or_else(|| format!("Failed to downcast column {name} to TimestampNanosecondArray"))?;
                    Ok(Some(arr.clone()))
                }
                _ => Err(format!(
                    "Column {name} has unexpected type: {:?}",
                    col.data_type()
                )),
            },
            None => Ok(None),
        }
    }

    fn get_binary_array(batch: &RecordBatch, name: &str) -> Option<StringArray> {
        batch.column_by_name(name).and_then(|col| {
            // Binary data in Arrow is often stored as LargeBinary or Binary
            // For trace_id/span_id, we'll convert to hex strings
            col.as_any()
                .downcast_ref::<arrow::array::BinaryArray>()
                .map(|arr| {
                    let strings: Vec<Option<String>> = (0..arr.len())
                        .map(|i| {
                            if arr.is_valid(i) {
                                Some(hex::encode(arr.value(i)))
                            } else {
                                None
                            }
                        })
                        .collect();
                    StringArray::from(strings)
                })
        })
    }

    fn get_u32_array(batch: &RecordBatch, name: &str) -> Option<UInt32Array> {
        batch
            .column_by_name(name)
            .and_then(|col| col.as_any().downcast_ref::<UInt32Array>())
            .cloned()
    }

    fn get_i32_array(batch: &RecordBatch, name: &str) -> Result<Int32Array, String> {
        batch
            .column_by_name(name)
            .and_then(|col| col.as_any().downcast_ref::<Int32Array>())
            .cloned()
            .ok_or_else(|| format!("Required column {name} not found or has wrong type"))
    }

    fn get_string_array(batch: &RecordBatch, name: &str) -> Option<StringArray> {
        batch
            .column_by_name(name)
            .and_then(|col| col.as_any().downcast_ref::<StringArray>())
            .cloned()
    }

    fn get_body_string(batch: &RecordBatch, name: &str) -> Option<StringArray> {
        // Body in OTAP is a struct with a value field
        // For now, we'll extract string values
        batch.column_by_name(name).and_then(|col| {
            col.as_any()
                .downcast_ref::<arrow::array::StructArray>()
                .and_then(|struct_arr| {
                    struct_arr
                        .column_by_name("string_value")
                        .and_then(|val_col| val_col.as_any().downcast_ref::<StringArray>())
                        .cloned()
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::TimestampNanosecondBuilder;
    use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
    use std::sync::Arc;

    #[test]
    fn test_encode_simple_log_batch() {
        // Create a simple Arrow RecordBatch with logs
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                COL_TIME_UNIX_NANO,
                DataType::Timestamp(TimeUnit::Nanosecond, None),
                true,
            ),
            Field::new(
                COL_OBSERVED_TIME_UNIX_NANO,
                DataType::Timestamp(TimeUnit::Nanosecond, None),
                true,
            ),
            Field::new(COL_SEVERITY_NUMBER, DataType::Int32, false),
            Field::new(COL_SEVERITY_TEXT, DataType::Utf8, true),
            Field::new(COL_EVENT_NAME, DataType::Utf8, true),
        ]));

        let mut time_builder = TimestampNanosecondBuilder::new();
        time_builder.append_value(1234567890000000000);
        let time_array = time_builder.finish();

        let mut obs_time_builder = TimestampNanosecondBuilder::new();
        obs_time_builder.append_value(1234567890000000000);
        let obs_time_array = obs_time_builder.finish();

        let severity_array = Int32Array::from(vec![9]);
        let severity_text_array = StringArray::from(vec![Some("INFO")]);
        let event_name_array = StringArray::from(vec![Some("TestEvent")]);

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(time_array),
                Arc::new(obs_time_array),
                Arc::new(severity_array),
                Arc::new(severity_text_array),
                Arc::new(event_name_array),
            ],
        )
        .unwrap();

        let encoder = OtapEncoder::new("test_metadata".to_string());
        let result = encoder.encode_log_batch(&batch);

        assert!(result.is_ok());
        let batches = result.unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].event_name, "TestEvent");
        assert!(!batches[0].data.is_empty());
    }
}
