use crate::client::EncodedBatch;
use crate::payload_encoder::bond_encoder::{BondDataType, BondEncodedSchema, BondWriter, FieldDef};
use crate::payload_encoder::central_blob::{
    BatchMetadata, CentralBlob, CentralEventEntry, CentralSchemaEntry,
};
use crate::payload_encoder::lz4_chunked_compression::lz4_chunked_compression;
use chrono::{TimeZone, Utc};
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::logs::v1::LogRecord;
use opentelemetry_proto::tonic::trace::v1::Span;
use std::borrow::Cow;
use std::sync::Arc;
use tracing::debug;

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

// Span-specific field constants
const FIELD_KIND: &str = "kind";
const FIELD_START_TIME: &str = "startTime";
const FIELD_SUCCESS: &str = "success";
const FIELD_TRACE_STATE: &str = "traceState";
const FIELD_PARENT_ID: &str = "parentId";
const FIELD_LINKS: &str = "links";
const FIELD_STATUS_MESSAGE: &str = "statusMessage";

/// Encoder to write OTLP payload in bond form.
#[derive(Clone)]
pub(crate) struct OtlpEncoder;

impl OtlpEncoder {
    pub(crate) fn new() -> Self {
        OtlpEncoder {}
    }

    /// Encode a batch of logs into a vector of (event_name, compressed_bytes, schema_ids, start_time_nanos, end_time_nanos)
    /// The returned `data` field contains LZ4 chunked compressed bytes.
    /// On compression failure, the error is returned (no logging, no fallback).
    pub(crate) fn encode_log_batch<'a, I>(
        &self,
        logs: I,
        metadata: &str,
    ) -> Result<Vec<EncodedBatch>, String>
    where
        I: IntoIterator<Item = &'a opentelemetry_proto::tonic::logs::v1::LogRecord>,
    {
        use std::collections::HashMap;

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

                // Pre-allocate capacity: Each MD5 hash is 32 hex chars + 1 semicolon (except last)
                // Total: (32 chars per hash * num_schemas) + (semicolons = num_schemas - 1)
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

        let mut batches: HashMap<&str, BatchData> = HashMap::new();

        for log_record in logs {
            // Get the timestamp - prefer time_unix_nano, fall back to observed_time_unix_nano if time_unix_nano is 0
            let timestamp = if log_record.time_unix_nano != 0 {
                log_record.time_unix_nano
            } else {
                log_record.observed_time_unix_nano
            };

            // Use string slice directly to avoid unnecessary allocations
            let event_name_str = if log_record.event_name.is_empty() {
                "Log"
            } else {
                log_record.event_name.as_str()
            };

            // 1. Get schema with optimized single-pass field collection and schema ID calculation
            let (field_info, schema_id) =
                Self::determine_fields_and_schema_id(log_record, event_name_str);

            // 2. Encode row
            let row_buffer = self.write_row_data(log_record, &field_info);
            let level = log_record.severity_number as u8;

            // 3. Create or get existing batch entry with metadata tracking
            let entry = batches.entry(event_name_str).or_insert_with(|| BatchData {
                schemas: Vec::new(),
                events: Vec::new(),
                metadata: BatchMetadata {
                    start_time: timestamp,
                    end_time: timestamp,
                    schema_ids: String::new(),
                },
            });

            // Update timestamp range
            if timestamp != 0 {
                entry.metadata.start_time = entry.metadata.start_time.min(timestamp);
                entry.metadata.end_time = entry.metadata.end_time.max(timestamp);
            }

            // 4. Add schema entry if not already present (multiple schemas per event_name batch)
            if !entry.schemas.iter().any(|s| s.id == schema_id) {
                let schema_entry = Self::create_schema(schema_id, field_info);
                entry.schemas.push(schema_entry);
            }

            // 5. Create CentralEventEntry directly (optimization: no intermediate EncodedRow)
            let central_event = CentralEventEntry {
                schema_id,
                level,
                event_name: Arc::new(event_name_str.to_string()),
                row: row_buffer,
            };
            entry.events.push(central_event);
        }

        // 6. Encode blobs (one per event_name, potentially multiple schemas per blob)
        let mut blobs = Vec::with_capacity(batches.len());
        for (batch_event_name, mut batch_data) in batches {
            let schema_ids_string = batch_data.format_schema_ids();
            batch_data.metadata.schema_ids = schema_ids_string;

            let schemas_count = batch_data.schemas.len();
            let events_count = batch_data.events.len();

            let blob = CentralBlob {
                version: 1,
                format: 2,
                metadata: metadata.to_string(),
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
                "Encoded log batch"
            );

            blobs.push(EncodedBatch {
                event_name: batch_event_name.to_string(),
                data: compressed,
                metadata: batch_data.metadata,
                row_count: events_count,
            });
        }
        Ok(blobs)
    }

    /// Encode a batch of spans into a single payload
    /// All spans are grouped into a single batch with event_name "Span" for routing
    /// The returned `data` field contains LZ4 chunked compressed bytes.
    /// On compression failure, the error is returned (no logging, no fallback).
    pub(crate) fn encode_span_batch<'a, I>(
        &self,
        spans: I,
        metadata: &str,
    ) -> Result<Vec<EncodedBatch>, String>
    where
        I: IntoIterator<Item = &'a Span>,
    {
        // All spans use "Span" as event name for routing - no grouping by span name
        const EVENT_NAME: &str = "Span";

        let mut schemas = Vec::new();
        let mut events = Vec::new();
        let mut start_time = u64::MAX;
        let mut end_time = 0u64;

        for span in spans {
            // 1. Get schema with optimized single-pass field collection and schema ID calculation
            let (field_info, schema_id) =
                Self::determine_span_fields_and_schema_id(span, EVENT_NAME);

            // 2. Encode row
            let row_buffer = self.write_span_row_data(span, &field_info);
            let level = 5; // Default level for spans (INFO equivalent)

            // 3. Update timestamp range
            if span.start_time_unix_nano != 0 {
                start_time = start_time.min(span.start_time_unix_nano);
            }
            if span.end_time_unix_nano != 0 {
                end_time = end_time.max(span.end_time_unix_nano);
            }

            // 4. Add schema entry if not already present
            // TODO - This can have collision if different spans have same schema ID but different fields
            if !schemas
                .iter()
                .any(|s: &CentralSchemaEntry| s.id == schema_id)
            {
                let schema_entry = Self::create_span_schema(schema_id, field_info);
                schemas.push(schema_entry);
            }

            // 5. Create CentralEventEntry
            let central_event = CentralEventEntry {
                schema_id,
                level,
                event_name: Arc::new(EVENT_NAME.to_string()),
                row: row_buffer,
            };
            events.push(central_event);
        }

        // Handle case with no spans
        if events.is_empty() {
            return Ok(Vec::new());
        }

        // Format schema IDs
        // TODO - this can be shared code with log batch
        let schema_ids_string = {
            use std::fmt::Write;
            if schemas.is_empty() {
                String::new()
            } else {
                let estimated_capacity = schemas.len() * 32 + schemas.len().saturating_sub(1);
                schemas.iter().enumerate().fold(
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
        };

        // Create single batch with all spans
        let schemas_count = schemas.len();
        let events_count = events.len();

        let batch_metadata = BatchMetadata {
            start_time: if start_time == u64::MAX {
                0
            } else {
                start_time
            },
            end_time,
            schema_ids: schema_ids_string,
        };

        let blob = CentralBlob {
            version: 1,
            format: 2,
            metadata: metadata.to_string(),
            schemas,
            events,
        };

        let uncompressed = blob.to_bytes();
        let compressed = lz4_chunked_compression(&uncompressed).map_err(|e| {
            debug!(
                name: "encoder.encode_span_batch.compress_error",
                target: "geneva-uploader",
                error = %e,
                "LZ4 compression failed for spans"
            );
            format!("compression failed: {e}")
        })?;

        debug!(
            name: "encoder.encode_span_batch",
            target: "geneva-uploader",
            event_name = EVENT_NAME,
            schemas = schemas_count,
            spans = events_count,
            uncompressed_size = uncompressed.len(),
            compressed_size = compressed.len(),
            "Encoded span batch"
        );

        Ok(vec![EncodedBatch {
            event_name: EVENT_NAME.to_string(),
            data: compressed,
            metadata: batch_metadata,
            row_count: events_count,
        }])
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
            fields.push((FIELD_TRACE_FLAGS.into(), BondDataType::BT_UINT32));
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

        // No sorting - field order affects schema ID calculation
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

    /// Determine span fields and calculate schema ID in a single pass for optimal performance
    fn determine_span_fields_and_schema_id(span: &Span, event_name: &str) -> (Vec<FieldDef>, u64) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Pre-allocate with estimated capacity to avoid reallocations
        let estimated_capacity = 15 + span.attributes.len(); // 7 always + 8 max conditional + attributes
        let mut fields = Vec::with_capacity(estimated_capacity);

        // Initialize hasher for schema ID calculation
        let mut hasher = DefaultHasher::new();
        event_name.hash(&mut hasher);

        // Part A - Always present fields for spans
        fields.push((Cow::Borrowed(FIELD_ENV_NAME), BondDataType::BT_STRING));
        fields.push((FIELD_ENV_VER.into(), BondDataType::BT_STRING));
        fields.push((FIELD_TIMESTAMP.into(), BondDataType::BT_STRING));
        fields.push((FIELD_ENV_TIME.into(), BondDataType::BT_STRING));

        // Span-specific required fields
        fields.push((FIELD_KIND.into(), BondDataType::BT_INT32));
        fields.push((FIELD_START_TIME.into(), BondDataType::BT_STRING));
        fields.push((FIELD_SUCCESS.into(), BondDataType::BT_BOOL));

        // Part A extension - Conditional fields
        if !span.trace_id.is_empty() {
            fields.push((FIELD_TRACE_ID.into(), BondDataType::BT_STRING));
        }
        if !span.span_id.is_empty() {
            fields.push((FIELD_SPAN_ID.into(), BondDataType::BT_STRING));
        }
        if span.flags != 0 {
            fields.push((FIELD_TRACE_FLAGS.into(), BondDataType::BT_UINT32));
        }

        // Part B - Span-specific optional fields
        if !span.name.is_empty() {
            fields.push((FIELD_NAME.into(), BondDataType::BT_STRING));
        }
        if !span.trace_state.is_empty() {
            fields.push((FIELD_TRACE_STATE.into(), BondDataType::BT_STRING));
        }
        if !span.parent_span_id.is_empty() {
            fields.push((FIELD_PARENT_ID.into(), BondDataType::BT_STRING));
        }
        if !span.links.is_empty() {
            fields.push((FIELD_LINKS.into(), BondDataType::BT_STRING));
        }
        if let Some(status) = &span.status {
            if !status.message.is_empty() {
                fields.push((FIELD_STATUS_MESSAGE.into(), BondDataType::BT_STRING));
            }
        }

        // Part C - Dynamic attributes
        for attr in &span.attributes {
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

    /// Create schema - always creates a new CentralSchemaEntry
    fn create_schema(schema_id: u64, field_info: Vec<FieldDef>) -> CentralSchemaEntry {
        let schema = BondEncodedSchema::from_fields("OtlpLogRecord", "telemetry", field_info); //TODO - use actual struct name and namespace

        let schema_bytes = schema.as_bytes();
        let schema_md5 = md5::compute(schema_bytes).0;

        CentralSchemaEntry {
            id: schema_id,
            md5: schema_md5,
            schema,
        }
    }

    /// Create span schema - always creates a new CentralSchemaEntry
    fn create_span_schema(schema_id: u64, field_info: Vec<FieldDef>) -> CentralSchemaEntry {
        let schema = BondEncodedSchema::from_fields("OtlpSpanRecord", "telemetry", field_info);

        let schema_bytes = schema.as_bytes();
        let schema_md5 = md5::compute(schema_bytes).0;

        CentralSchemaEntry {
            id: schema_id,
            md5: schema_md5,
            schema,
        }
    }

    /// Write span row data directly from Span
    // TODO - code duplication between write_span_row_data() and write_row_data() - consider extracting common field handling
    fn write_span_row_data(&self, span: &Span, fields: &[FieldDef]) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(fields.len() * 50);

        // Pre-calculate timestamp (use start time as primary timestamp for both fields)
        let formatted_timestamp = Self::format_timestamp(span.start_time_unix_nano);

        for field in fields {
            match field.name.as_ref() {
                FIELD_ENV_NAME => BondWriter::write_string(&mut buffer, "TestEnv"), // TODO - placeholder
                FIELD_ENV_VER => BondWriter::write_string(&mut buffer, "4.0"), // TODO - placeholder
                FIELD_TIMESTAMP | FIELD_ENV_TIME => {
                    BondWriter::write_string(&mut buffer, &formatted_timestamp);
                }
                FIELD_KIND => {
                    BondWriter::write_numeric(&mut buffer, span.kind);
                }
                FIELD_START_TIME => {
                    BondWriter::write_string(&mut buffer, &formatted_timestamp);
                }
                FIELD_SUCCESS => {
                    // Determine success based on status
                    let success = match &span.status {
                        Some(status) => {
                            use opentelemetry_proto::tonic::trace::v1::status::StatusCode;
                            match StatusCode::try_from(status.code) {
                                Ok(StatusCode::Ok) => true,
                                Ok(StatusCode::Error) => false,
                                _ => true, // Unset or unknown defaults to true
                            }
                        }
                        None => true, // No status defaults to true
                    };
                    BondWriter::write_bool(&mut buffer, success);
                }
                FIELD_TRACE_ID => {
                    let hex_bytes = Self::encode_id_to_hex::<32>(&span.trace_id);
                    let hex_str = std::str::from_utf8(&hex_bytes).unwrap();
                    BondWriter::write_string(&mut buffer, hex_str);
                }
                FIELD_SPAN_ID => {
                    let hex_bytes = Self::encode_id_to_hex::<16>(&span.span_id);
                    let hex_str = std::str::from_utf8(&hex_bytes).unwrap();
                    BondWriter::write_string(&mut buffer, hex_str);
                }
                FIELD_TRACE_FLAGS => {
                    BondWriter::write_numeric(&mut buffer, span.flags);
                }
                FIELD_NAME => {
                    BondWriter::write_string(&mut buffer, &span.name);
                }
                FIELD_TRACE_STATE => {
                    BondWriter::write_string(&mut buffer, &span.trace_state);
                }
                FIELD_PARENT_ID => {
                    let hex_bytes = Self::encode_id_to_hex::<16>(&span.parent_span_id);
                    let hex_str = std::str::from_utf8(&hex_bytes).unwrap();
                    BondWriter::write_string(&mut buffer, hex_str);
                }
                FIELD_LINKS => {
                    // Manual JSON building to avoid intermediate allocations
                    let links_json = Self::serialize_links(&span.links);
                    BondWriter::write_string(&mut buffer, &links_json);
                }
                FIELD_STATUS_MESSAGE => {
                    if let Some(status) = &span.status {
                        BondWriter::write_string(&mut buffer, &status.message);
                    }
                }
                _ => {
                    // Handle dynamic attributes
                    // TODO - optimize better - we could update determine_fields to also return a vec of bytes which has bond serialized attributes
                    if let Some(attr) = span.attributes.iter().find(|a| a.key == field.name) {
                        self.write_attribute_value(&mut buffer, attr, field.type_id);
                    }
                }
            }
        }

        buffer
    }

    /// Write row data directly from LogRecord
    fn write_row_data(&self, log: &LogRecord, sorted_fields: &[FieldDef]) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(sorted_fields.len() * 50); //TODO - estimate better

        // Pre-calculate timestamp to avoid duplicate computation for FIELD_TIMESTAMP and FIELD_ENV_TIME
        let formatted_timestamp = {
            let timestamp_nanos = if log.time_unix_nano != 0 {
                log.time_unix_nano
            } else {
                log.observed_time_unix_nano
            };
            Self::format_timestamp(timestamp_nanos)
        };

        for field in sorted_fields {
            match field.name.as_ref() {
                FIELD_ENV_NAME => BondWriter::write_string(&mut buffer, "TestEnv"), // TODO - placeholder for actual env name
                FIELD_ENV_VER => BondWriter::write_string(&mut buffer, "4.0"), // TODO - placeholder for actual env version
                FIELD_TIMESTAMP | FIELD_ENV_TIME => {
                    BondWriter::write_string(&mut buffer, &formatted_timestamp);
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
                    BondWriter::write_numeric(&mut buffer, log.flags);
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

    /// Links serialization
    fn serialize_links(links: &[opentelemetry_proto::tonic::trace::v1::span::Link]) -> String {
        if links.is_empty() {
            return "[]".to_string();
        }

        // Estimate capacity: Each link needs ~80 chars for JSON structure + 32 chars for trace_id + 16 chars for span_id
        // JSON overhead: {"toSpanId":"","toTraceId":""} = ~30 chars + commas/brackets
        let estimated_capacity = links.len() * 128 + 2; // Extra buffer for safety
        let mut json = String::with_capacity(estimated_capacity);

        json.push('[');

        for (i, link) in links.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }

            json.push_str(r#"{"toSpanId":""#);

            // Write hex directly to avoid temporary string allocation
            for &byte in &link.span_id {
                json.push_str(&format!("{:02x}", byte));
            }

            json.push_str(r#"","toTraceId":""#);

            // Write hex directly to avoid temporary string allocation
            for &byte in &link.trace_id {
                json.push_str(&format!("{:02x}", byte));
            }

            json.push_str(r#""}"#);
        }

        json.push(']');
        json
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
        let result = encoder.encode_log_batch([log].iter(), metadata).unwrap();

        assert!(!result.is_empty());
    }

    #[test]
    fn test_multiple_schemas_per_batch() {
        let encoder = OtlpEncoder::new();

        // Create multiple log records with different schema structures
        // to test that multiple schemas can exist within the same batch
        let log1 = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: "user_action".to_string(),
            severity_number: 9,
            severity_text: "INFO".to_string(),
            ..Default::default()
        };

        // Schema 2: Same event_name but with trace_id (different schema)
        let mut log2 = LogRecord {
            event_name: "user_action".to_string(),
            observed_time_unix_nano: 1_700_000_001_000_000_000,
            severity_number: 10,
            severity_text: "WARN".to_string(),
            ..Default::default()
        };
        log2.trace_id = vec![1; 16];

        // Schema 3: Same event_name but with attributes (different schema)
        let mut log3 = LogRecord {
            event_name: "user_action".to_string(),
            observed_time_unix_nano: 1_700_000_002_000_000_000,
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

        let metadata = "namespace=test";

        // Encode multiple log records with different schema structures but same event_name
        let result = encoder
            .encode_log_batch([log1, log2, log3].iter(), metadata)
            .unwrap();

        // Should create one batch (same event_name = "user_action")
        assert_eq!(result.len(), 1);
        let batch = &result[0];
        assert_eq!(batch.event_name, "user_action");

        // Verify that multiple schemas were created within the same batch
        // schema_ids should contain multiple semicolon-separated MD5 hashes
        let schema_ids = &batch.metadata.schema_ids;
        assert!(!schema_ids.is_empty());

        // Split by semicolon to get individual schema IDs
        let schema_id_list: Vec<&str> = schema_ids.split(';').collect();

        // Should have 3 different schema IDs (one per unique schema structure)
        assert_eq!(
            schema_id_list.len(),
            3,
            "Expected 3 schema IDs but found {}: {}",
            schema_id_list.len(),
            schema_ids
        );

        // Verify all schema IDs are different (each log record has different schema structure)
        let unique_schemas: std::collections::HashSet<&str> = schema_id_list.into_iter().collect();
        assert_eq!(
            unique_schemas.len(),
            3,
            "Expected 3 unique schema IDs but found duplicates in: {schema_ids}"
        );

        // Verify each schema ID is a valid MD5 hash (32 hex characters)
        for schema_id in unique_schemas {
            assert_eq!(
                schema_id.len(),
                32,
                "Schema ID should be 32 hex characters: {schema_id}"
            );
            assert!(
                schema_id.chars().all(|c| c.is_ascii_hexdigit()),
                "Schema ID should contain only hex characters: {schema_id}"
            );
        }
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

        let result = encoder.encode_log_batch([log].iter(), "test").unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event_name, "test_event");
        assert!(!result[0].data.is_empty()); // Should have encoded data
        assert!(!result[0].metadata.schema_ids.is_empty()); // schema_ids should not be empty
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

        let result = encoder
            .encode_log_batch([log1, log2, log3].iter(), "test")
            .unwrap();

        // All should be in one batch with same event_name
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event_name, "user_action");
        assert!(!result[0].data.is_empty()); // Should have encoded data
                                             // Should have 3 different schema IDs (semicolon-separated)
        assert_eq!(result[0].metadata.schema_ids.matches(';').count(), 2); // 3 schemas = 2 semicolons
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

        let result = encoder
            .encode_log_batch([log1, log2].iter(), "test")
            .unwrap();

        // Should create 2 separate batches
        assert_eq!(result.len(), 2);

        let event_names: Vec<&String> = result.iter().map(|batch| &batch.event_name).collect();
        assert!(event_names.contains(&&"login".to_string()));
        assert!(event_names.contains(&&"logout".to_string()));

        // Each batch should have 1 event (verify by checking data is not empty)
        assert!(result.iter().all(|batch| !batch.data.is_empty()));
    }

    #[test]
    fn test_empty_event_name_defaults_to_log() {
        let encoder = OtlpEncoder::new();

        let log = LogRecord {
            event_name: "".to_string(),
            severity_number: 9,
            ..Default::default()
        };

        let result = encoder.encode_log_batch([log].iter(), "test").unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event_name, "Log"); // Should default to "Log"
        assert!(!result[0].data.is_empty()); // Should have encoded data
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

        let result = encoder
            .encode_log_batch([log1, log2, log3, log4].iter(), "test")
            .unwrap();

        // Should create 3 batches: "user_action", "system_alert", "Log"
        assert_eq!(result.len(), 3);

        // Find each batch and verify counts
        let user_action = result
            .iter()
            .find(|batch| batch.event_name == "user_action")
            .unwrap();
        let system_alert = result
            .iter()
            .find(|batch| batch.event_name == "system_alert")
            .unwrap();
        let log_batch = result
            .iter()
            .find(|batch| batch.event_name == "Log")
            .unwrap();

        // Verify that each batch has data and schema IDs
        assert!(!user_action.data.is_empty()); // Should have encoded data
        assert_eq!(user_action.metadata.schema_ids.matches(';').count(), 1); // 2 schemas = 1 semicolon
        assert!(!system_alert.data.is_empty()); // Should have encoded data
        assert_eq!(system_alert.metadata.schema_ids.matches(';').count(), 0); // 1 schema = 0 semicolons
        assert!(!log_batch.data.is_empty()); // Should have encoded data
        assert_eq!(log_batch.metadata.schema_ids.matches(';').count(), 0); // 1 schema = 0 semicolons
    }

    #[test]
    fn test_span_encoding() {
        let encoder = OtlpEncoder::new();

        let mut span = Span {
            trace_id: vec![1; 16],
            span_id: vec![2; 8],
            parent_span_id: vec![3; 8],
            name: "test_span".to_string(),
            kind: 1, // CLIENT
            start_time_unix_nano: 1_700_000_000_000_000_000,
            end_time_unix_nano: 1_700_000_001_000_000_000,
            flags: 1,
            trace_state: "key=value".to_string(),
            ..Default::default()
        };

        // Add some attributes
        span.attributes.push(KeyValue {
            key: "http.method".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("GET".to_string())),
            }),
        });

        span.attributes.push(KeyValue {
            key: "http.status_code".to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(200)),
            }),
        });

        let metadata = "namespace=testNamespace/eventVersion=Ver1v0";
        let result = encoder.encode_span_batch([span].iter(), metadata).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event_name, "Span"); // All spans use "Span" event name for routing
        assert!(!result[0].data.is_empty());
    }

    #[test]
    fn test_span_with_links() {
        use opentelemetry_proto::tonic::trace::v1::span::Link;

        let encoder = OtlpEncoder::new();

        let mut span = Span {
            trace_id: vec![1; 16],
            span_id: vec![2; 8],
            name: "linked_span".to_string(),
            kind: 2, // SERVER
            start_time_unix_nano: 1_700_000_000_000_000_000,
            end_time_unix_nano: 1_700_000_001_000_000_000,
            ..Default::default()
        };

        // Add some links
        span.links.push(Link {
            trace_id: vec![4; 16],
            span_id: vec![5; 8],
            ..Default::default()
        });

        span.links.push(Link {
            trace_id: vec![6; 16],
            span_id: vec![7; 8],
            ..Default::default()
        });

        let result = encoder.encode_span_batch([span].iter(), "test").unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event_name, "Span"); // All spans use "Span" event name for routing
        assert!(!result[0].data.is_empty());
    }

    #[test]
    fn test_span_with_status() {
        use opentelemetry_proto::tonic::trace::v1::{status::StatusCode, Status};

        let encoder = OtlpEncoder::new();

        let mut span = Span {
            trace_id: vec![1; 16],
            span_id: vec![2; 8],
            name: "error_span".to_string(),
            kind: 1,
            start_time_unix_nano: 1_700_000_000_000_000_000,
            end_time_unix_nano: 1_700_000_001_000_000_000,
            ..Default::default()
        };

        span.status = Some(Status {
            message: "Something went wrong".to_string(),
            code: StatusCode::Error as i32,
        });

        let result = encoder.encode_span_batch([span].iter(), "test").unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event_name, "Span"); // All spans use "Span" event name for routing
        assert!(!result[0].data.is_empty());
    }

    #[test]
    fn test_multiple_spans_same_name() {
        let encoder = OtlpEncoder::new();

        let span1 = Span {
            trace_id: vec![1; 16],
            span_id: vec![2; 8],
            name: "database_query".to_string(),
            kind: 3, // CLIENT
            start_time_unix_nano: 1_700_000_000_000_000_000,
            end_time_unix_nano: 1_700_000_001_000_000_000,
            ..Default::default()
        };

        let span2 = Span {
            trace_id: vec![3; 16],
            span_id: vec![4; 8],
            name: "database_query".to_string(), // Same name as span1
            kind: 3,
            start_time_unix_nano: 1_700_000_002_000_000_000,
            end_time_unix_nano: 1_700_000_003_000_000_000,
            ..Default::default()
        };

        // Verify that both spans have name field in schema
        let (fields1, _) = OtlpEncoder::determine_span_fields_and_schema_id(&span1, "Span");
        let name_field_present1 = fields1
            .iter()
            .any(|field| field.name.as_ref() == FIELD_NAME);
        assert!(
            name_field_present1,
            "Span with non-empty name should include 'name' field in schema"
        );

        let (fields2, _) = OtlpEncoder::determine_span_fields_and_schema_id(&span2, "Span");
        let name_field_present2 = fields2
            .iter()
            .any(|field| field.name.as_ref() == FIELD_NAME);
        assert!(
            name_field_present2,
            "Span with non-empty name should include 'name' field in schema"
        );

        let result = encoder
            .encode_span_batch([span1, span2].iter(), "test")
            .unwrap();

        // Should create one batch with same event_name
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event_name, "Span"); // All spans use "Span" event name for routing
        assert!(!result[0].data.is_empty());
        // Should have 1 schema ID since both spans have same schema structure
        assert_eq!(result[0].metadata.schema_ids.matches(';').count(), 0); // 1 schema = 0 semicolons
    }

    #[test]
    fn test_optimized_links_serialization() {
        use opentelemetry_proto::tonic::trace::v1::span::Link;

        // Test that optimized serialization produces correct JSON output
        let links = vec![
            Link {
                trace_id: vec![
                    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                    0xab, 0xcd, 0xef,
                ],
                span_id: vec![0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10],
                ..Default::default()
            },
            Link {
                trace_id: vec![
                    0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                    0xee, 0xff, 0x00,
                ],
                span_id: vec![0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77],
                ..Default::default()
            },
        ];

        let result = OtlpEncoder::serialize_links(&links);

        // Verify JSON structure is correct
        assert!(result.starts_with('['));
        assert!(result.ends_with(']'));
        assert!(result.contains("toSpanId"));
        assert!(result.contains("toTraceId"));

        // Verify it contains the expected hex values
        assert!(result.contains("fedcba9876543210")); // First span_id
        assert!(result.contains("0123456789abcdef0123456789abcdef")); // First trace_id
        assert!(result.contains("0011223344556677")); // Second span_id
        assert!(result.contains("112233445566778899aabbccddeeff00")); // Second trace_id

        // Test empty links
        let empty_result = OtlpEncoder::serialize_links(&[]);
        assert_eq!(empty_result, "[]");

        // Test single link
        let single_link = vec![Link {
            trace_id: vec![0x12; 16],
            span_id: vec![0x34; 8],
            ..Default::default()
        }];
        let single_result = OtlpEncoder::serialize_links(&single_link);
        assert!(single_result.contains("3434343434343434")); // span_id
        assert!(single_result.contains("12121212121212121212121212121212")); // trace_id
                                                                             // Single item should have one comma (between toSpanId and toTraceId) but no comma between items
        assert_eq!(single_result.matches(',').count(), 1); // Only one comma for field separation
        assert!(single_result.starts_with('['));
        assert!(single_result.ends_with(']'));
    }
}
