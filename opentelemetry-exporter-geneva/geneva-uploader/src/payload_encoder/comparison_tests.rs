//! Comparison tests between OTAP and OTLP encoders
//!
//! These tests validate that both encoders produce semantically equivalent output
//! when given the same logical data in their respective formats (Arrow vs Protobuf).

#[cfg(test)]
mod tests {
    use crate::payload_encoder::lz4_chunked_compression::lz4_decompress;
    use crate::payload_encoder::otap_encoder::OtapEncoder;
    use crate::payload_encoder::otlp_encoder::OtlpEncoder;
    use arrow::array::{
        ArrayRef, BinaryBuilder, Int32Array, RecordBatch, StringArray, StructArray,
        TimestampNanosecondBuilder, UInt32Array,
    };
    use arrow::datatypes::{DataType, Field, Fields, Schema, TimeUnit};
    use opentelemetry_proto::tonic::common::v1::{any_value::Value, AnyValue};
    use opentelemetry_proto::tonic::logs::v1::LogRecord;
    use std::sync::Arc;

    // Constants for test data
    const TEST_TIMESTAMP: u64 = 1_700_000_000_000_000_000;
    const TEST_OBSERVED_TIMESTAMP: u64 = 1_700_000_001_000_000_000;
    const TEST_SEVERITY_NUMBER: i32 = 9;
    const TEST_SEVERITY_TEXT: &str = "INFO";
    const TEST_EVENT_NAME: &str = "TestEvent";
    const TEST_BODY: &str = "Test log message";
    const TEST_METADATA: &str = "namespace=testNamespace/eventVersion=Ver1v0";

    /// Helper to create a basic OTLP LogRecord
    fn create_otlp_log_basic() -> LogRecord {
        LogRecord {
            time_unix_nano: TEST_TIMESTAMP,
            observed_time_unix_nano: TEST_OBSERVED_TIMESTAMP,
            severity_number: TEST_SEVERITY_NUMBER,
            severity_text: TEST_SEVERITY_TEXT.to_string(),
            event_name: TEST_EVENT_NAME.to_string(),
            body: Some(AnyValue {
                value: Some(Value::StringValue(TEST_BODY.to_string())),
            }),
            ..Default::default()
        }
    }

    /// Helper to create a basic OTAP Arrow RecordBatch
    fn create_otap_log_basic() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "time_unix_nano",
                DataType::Timestamp(TimeUnit::Nanosecond, None),
                true,
            ),
            Field::new(
                "observed_time_unix_nano",
                DataType::Timestamp(TimeUnit::Nanosecond, None),
                true,
            ),
            Field::new("severity_number", DataType::Int32, false),
            Field::new("severity_text", DataType::Utf8, true),
            Field::new("event_name", DataType::Utf8, true),
            Field::new(
                "body",
                DataType::Struct(Fields::from(vec![Field::new(
                    "string_value",
                    DataType::Utf8,
                    true,
                )])),
                true,
            ),
        ]));

        let mut time_builder = TimestampNanosecondBuilder::new();
        time_builder.append_value(TEST_TIMESTAMP as i64);
        let time_array = time_builder.finish();

        let mut obs_time_builder = TimestampNanosecondBuilder::new();
        obs_time_builder.append_value(TEST_OBSERVED_TIMESTAMP as i64);
        let obs_time_array = obs_time_builder.finish();

        let severity_array = Int32Array::from(vec![TEST_SEVERITY_NUMBER]);
        let severity_text_array = StringArray::from(vec![Some(TEST_SEVERITY_TEXT)]);
        let event_name_array = StringArray::from(vec![Some(TEST_EVENT_NAME)]);

        // Create body struct array
        let string_value_array = StringArray::from(vec![Some(TEST_BODY)]);
        let body_array = StructArray::from(vec![(
            Arc::new(Field::new("string_value", DataType::Utf8, true)),
            Arc::new(string_value_array) as ArrayRef,
        )]);

        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(time_array),
                Arc::new(obs_time_array),
                Arc::new(severity_array),
                Arc::new(severity_text_array),
                Arc::new(event_name_array),
                Arc::new(body_array),
            ],
        )
        .unwrap()
    }

    /// Helper to create OTLP log with trace context
    fn create_otlp_log_with_trace() -> LogRecord {
        let mut log = create_otlp_log_basic();
        log.trace_id = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10];
        log.span_id = vec![0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18];
        log.flags = 1;
        log
    }

    /// Helper to create OTAP log with trace context
    fn create_otap_log_with_trace() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "time_unix_nano",
                DataType::Timestamp(TimeUnit::Nanosecond, None),
                true,
            ),
            Field::new(
                "observed_time_unix_nano",
                DataType::Timestamp(TimeUnit::Nanosecond, None),
                true,
            ),
            Field::new("severity_number", DataType::Int32, false),
            Field::new("severity_text", DataType::Utf8, true),
            Field::new("event_name", DataType::Utf8, true),
            Field::new(
                "body",
                DataType::Struct(Fields::from(vec![Field::new(
                    "string_value",
                    DataType::Utf8,
                    true,
                )])),
                true,
            ),
            Field::new("trace_id", DataType::Binary, true),
            Field::new("span_id", DataType::Binary, true),
            Field::new("flags", DataType::UInt32, true),
        ]));

        let mut time_builder = TimestampNanosecondBuilder::new();
        time_builder.append_value(TEST_TIMESTAMP as i64);
        let time_array = time_builder.finish();

        let mut obs_time_builder = TimestampNanosecondBuilder::new();
        obs_time_builder.append_value(TEST_OBSERVED_TIMESTAMP as i64);
        let obs_time_array = obs_time_builder.finish();

        let severity_array = Int32Array::from(vec![TEST_SEVERITY_NUMBER]);
        let severity_text_array = StringArray::from(vec![Some(TEST_SEVERITY_TEXT)]);
        let event_name_array = StringArray::from(vec![Some(TEST_EVENT_NAME)]);

        // Create body struct array
        let string_value_array = StringArray::from(vec![Some(TEST_BODY)]);
        let body_array = StructArray::from(vec![(
            Arc::new(Field::new("string_value", DataType::Utf8, true)),
            Arc::new(string_value_array) as ArrayRef,
        )]);

        // Create trace_id array
        let mut trace_id_builder = BinaryBuilder::new();
        trace_id_builder.append_value(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10]);
        let trace_id_array = trace_id_builder.finish();

        // Create span_id array
        let mut span_id_builder = BinaryBuilder::new();
        span_id_builder.append_value(&[0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18]);
        let span_id_array = span_id_builder.finish();

        let flags_array = UInt32Array::from(vec![Some(1)]);

        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(time_array),
                Arc::new(obs_time_array),
                Arc::new(severity_array),
                Arc::new(severity_text_array),
                Arc::new(event_name_array),
                Arc::new(body_array),
                Arc::new(trace_id_array),
                Arc::new(span_id_array),
                Arc::new(flags_array),
            ],
        )
        .unwrap()
    }

    /// Helper to decompress LZ4 data
    fn decompress_batch(compressed: &[u8]) -> Vec<u8> {
        lz4_decompress(compressed).expect("Failed to decompress")
    }

    /// Extract basic metadata for comparison
    #[derive(Debug)]
    struct EncodedMetadata {
        event_name: String,
        uncompressed_size: usize,
        compressed_size: usize,
        start_time: u64,
        end_time: u64,
        schema_count: usize,
    }

    impl EncodedMetadata {
        fn from_batch(batch: &crate::client::EncodedBatch) -> Self {
            let uncompressed = decompress_batch(&batch.data);
            // Count schemas by counting semicolons + 1
            let schema_count = if batch.metadata.schema_ids.is_empty() {
                0
            } else {
                batch.metadata.schema_ids.matches(';').count() + 1
            };

            EncodedMetadata {
                event_name: batch.event_name.clone(),
                uncompressed_size: uncompressed.len(),
                compressed_size: batch.data.len(),
                start_time: batch.metadata.start_time,
                end_time: batch.metadata.end_time,
                schema_count,
            }
        }
    }

    #[test]
    fn test_basic_log_encoding_comparison() {
        let otlp_encoder = OtlpEncoder::new();
        let otap_encoder = OtapEncoder::new(TEST_METADATA.to_string());

        let otlp_log = create_otlp_log_basic();
        let otap_batch = create_otap_log_basic();

        let otlp_result = otlp_encoder
            .encode_log_batch([otlp_log].iter(), TEST_METADATA)
            .expect("OTLP encoding failed");
        let otap_result = otap_encoder
            .encode_log_batch(&otap_batch)
            .expect("OTAP encoding failed");

        assert_eq!(otlp_result.len(), 1, "OTLP should produce 1 batch");
        assert_eq!(otap_result.len(), 1, "OTAP should produce 1 batch");

        let otlp_meta = EncodedMetadata::from_batch(&otlp_result[0]);
        let otap_meta = EncodedMetadata::from_batch(&otap_result[0]);

        // Compare metadata
        assert_eq!(
            otlp_meta.event_name, otap_meta.event_name,
            "Event names should match"
        );
        assert_eq!(
            otlp_meta.start_time, otap_meta.start_time,
            "Start times should match"
        );
        assert_eq!(
            otlp_meta.end_time, otap_meta.end_time,
            "End times should match"
        );
        assert_eq!(
            otlp_meta.schema_count, otap_meta.schema_count,
            "Schema counts should match"
        );

        // Validate both produced compressed output
        assert!(
            otlp_meta.compressed_size > 0,
            "OTLP should produce compressed data"
        );
        assert!(
            otap_meta.compressed_size > 0,
            "OTAP should produce compressed data"
        );

        println!("Basic Log Encoding Comparison:");
        println!("  OTLP - Compressed: {} bytes, Uncompressed: {} bytes, Compression ratio: {:.2}%",
            otlp_meta.compressed_size, otlp_meta.uncompressed_size,
            (otlp_meta.compressed_size as f64 / otlp_meta.uncompressed_size as f64) * 100.0);
        println!("  OTAP - Compressed: {} bytes, Uncompressed: {} bytes, Compression ratio: {:.2}%",
            otap_meta.compressed_size, otap_meta.uncompressed_size,
            (otap_meta.compressed_size as f64 / otap_meta.uncompressed_size as f64) * 100.0);
    }

    #[test]
    fn test_trace_context_encoding_comparison() {
        let otlp_encoder = OtlpEncoder::new();
        let otap_encoder = OtapEncoder::new(TEST_METADATA.to_string());

        let otlp_log = create_otlp_log_with_trace();
        let otap_batch = create_otap_log_with_trace();

        let otlp_result = otlp_encoder
            .encode_log_batch([otlp_log].iter(), TEST_METADATA)
            .expect("OTLP encoding failed");
        let otap_result = otap_encoder
            .encode_log_batch(&otap_batch)
            .expect("OTAP encoding failed");

        assert_eq!(otlp_result.len(), 1, "OTLP should produce 1 batch");
        assert_eq!(otap_result.len(), 1, "OTAP should produce 1 batch");

        let otlp_meta = EncodedMetadata::from_batch(&otlp_result[0]);
        let otap_meta = EncodedMetadata::from_batch(&otap_result[0]);

        // Compare metadata
        assert_eq!(
            otlp_meta.event_name, otap_meta.event_name,
            "Event names should match"
        );
        assert_eq!(
            otlp_meta.schema_count, otap_meta.schema_count,
            "Schema counts should match (both should have trace fields)"
        );

        println!("Trace Context Encoding Comparison:");
        println!("  OTLP - Compressed: {} bytes, Uncompressed: {} bytes",
            otlp_meta.compressed_size, otlp_meta.uncompressed_size);
        println!("  OTAP - Compressed: {} bytes, Uncompressed: {} bytes",
            otap_meta.compressed_size, otap_meta.uncompressed_size);
    }

    #[test]
    fn test_multiple_logs_same_schema() {
        let otlp_encoder = OtlpEncoder::new();
        let otap_encoder = OtapEncoder::new(TEST_METADATA.to_string());

        // Create multiple OTLP logs with same schema
        let otlp_logs: Vec<LogRecord> = (0..10)
            .map(|i| {
                let mut log = create_otlp_log_basic();
                log.time_unix_nano = TEST_TIMESTAMP + i * 1_000_000_000;
                log.observed_time_unix_nano = TEST_OBSERVED_TIMESTAMP + i * 1_000_000_000;
                log
            })
            .collect();

        // Create equivalent OTAP batch with multiple rows
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "time_unix_nano",
                DataType::Timestamp(TimeUnit::Nanosecond, None),
                true,
            ),
            Field::new(
                "observed_time_unix_nano",
                DataType::Timestamp(TimeUnit::Nanosecond, None),
                true,
            ),
            Field::new("severity_number", DataType::Int32, false),
            Field::new("severity_text", DataType::Utf8, true),
            Field::new("event_name", DataType::Utf8, true),
            Field::new(
                "body",
                DataType::Struct(Fields::from(vec![Field::new(
                    "string_value",
                    DataType::Utf8,
                    true,
                )])),
                true,
            ),
        ]));

        let mut time_builder = TimestampNanosecondBuilder::new();
        let mut obs_time_builder = TimestampNanosecondBuilder::new();
        for i in 0..10 {
            time_builder.append_value((TEST_TIMESTAMP + i * 1_000_000_000) as i64);
            obs_time_builder.append_value((TEST_OBSERVED_TIMESTAMP + i * 1_000_000_000) as i64);
        }

        let severity_array = Int32Array::from(vec![TEST_SEVERITY_NUMBER; 10]);
        let severity_text_array = StringArray::from(vec![Some(TEST_SEVERITY_TEXT); 10]);
        let event_name_array = StringArray::from(vec![Some(TEST_EVENT_NAME); 10]);

        let string_value_array = StringArray::from(vec![Some(TEST_BODY); 10]);
        let body_array = StructArray::from(vec![(
            Arc::new(Field::new("string_value", DataType::Utf8, true)),
            Arc::new(string_value_array) as ArrayRef,
        )]);

        let otap_batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(time_builder.finish()),
                Arc::new(obs_time_builder.finish()),
                Arc::new(severity_array),
                Arc::new(severity_text_array),
                Arc::new(event_name_array),
                Arc::new(body_array),
            ],
        )
        .unwrap();

        let otlp_result = otlp_encoder
            .encode_log_batch(otlp_logs.iter(), TEST_METADATA)
            .expect("OTLP encoding failed");
        let otap_result = otap_encoder
            .encode_log_batch(&otap_batch)
            .expect("OTAP encoding failed");

        assert_eq!(otlp_result.len(), 1);
        assert_eq!(otap_result.len(), 1);

        let otlp_meta = EncodedMetadata::from_batch(&otlp_result[0]);
        let otap_meta = EncodedMetadata::from_batch(&otap_result[0]);

        // Both should have same event name and single schema
        assert_eq!(otlp_meta.event_name, otap_meta.event_name);
        assert_eq!(
            otlp_meta.schema_count, 1,
            "OTLP should have 1 schema for same-schema logs"
        );
        assert_eq!(
            otap_meta.schema_count, 1,
            "OTAP should have 1 schema for same-schema logs"
        );

        // Verify timestamp ranges
        assert_eq!(otlp_meta.start_time, TEST_TIMESTAMP);
        assert_eq!(otap_meta.start_time, TEST_TIMESTAMP);
        // End time should be the last timestamp (either time_unix_nano or observed_time_unix_nano)
        // For OTLP: uses time_unix_nano when non-zero, falls back to observed_time_unix_nano
        // Last log has time_unix_nano = TEST_TIMESTAMP + 9 * 1_000_000_000
        assert_eq!(otlp_meta.end_time, TEST_TIMESTAMP + 9 * 1_000_000_000);
        assert_eq!(otap_meta.end_time, TEST_TIMESTAMP + 9 * 1_000_000_000);

        println!("Multiple Logs (Same Schema) Comparison:");
        println!("  OTLP - Compressed: {} bytes, Uncompressed: {} bytes",
            otlp_meta.compressed_size, otlp_meta.uncompressed_size);
        println!("  OTAP - Compressed: {} bytes, Uncompressed: {} bytes",
            otap_meta.compressed_size, otap_meta.uncompressed_size);
        println!("  Size difference: {} bytes ({:.2}% difference)",
            (otlp_meta.compressed_size as i64 - otap_meta.compressed_size as i64).abs(),
            ((otlp_meta.compressed_size as f64 - otap_meta.compressed_size as f64).abs()
                / otlp_meta.compressed_size as f64) * 100.0);
    }

    #[test]
    fn test_empty_batch_handling() {
        let otlp_encoder = OtlpEncoder::new();
        let otap_encoder = OtapEncoder::new(TEST_METADATA.to_string());

        // Empty OTLP batch
        let otlp_result = otlp_encoder
            .encode_log_batch(std::iter::empty(), TEST_METADATA)
            .expect("OTLP encoding failed");

        // Empty OTAP batch
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "time_unix_nano",
                DataType::Timestamp(TimeUnit::Nanosecond, None),
                true,
            ),
            Field::new("severity_number", DataType::Int32, false),
            Field::new("event_name", DataType::Utf8, true),
        ]));

        let empty_batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(TimestampNanosecondBuilder::new().finish()),
                Arc::new(Int32Array::from(Vec::<i32>::new())),
                Arc::new(StringArray::from(Vec::<Option<&str>>::new())),
            ],
        )
        .unwrap();

        let otap_result = otap_encoder
            .encode_log_batch(&empty_batch)
            .expect("OTAP encoding failed");

        // Both should handle empty batches
        assert_eq!(otlp_result.len(), 0, "OTLP should return empty result");
        assert_eq!(otap_result.len(), 0, "OTAP should return empty result");
    }

    #[test]
    fn test_event_name_grouping() {
        let otlp_encoder = OtlpEncoder::new();
        let otap_encoder = OtapEncoder::new(TEST_METADATA.to_string());

        // Create OTLP logs with different event names
        let mut log1 = create_otlp_log_basic();
        log1.event_name = "EventA".to_string();

        let mut log2 = create_otlp_log_basic();
        log2.event_name = "EventB".to_string();

        let otlp_result = otlp_encoder
            .encode_log_batch([log1, log2].iter(), TEST_METADATA)
            .expect("OTLP encoding failed");

        // Create equivalent OTAP batch with multiple event names
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "time_unix_nano",
                DataType::Timestamp(TimeUnit::Nanosecond, None),
                true,
            ),
            Field::new(
                "observed_time_unix_nano",
                DataType::Timestamp(TimeUnit::Nanosecond, None),
                true,
            ),
            Field::new("severity_number", DataType::Int32, false),
            Field::new("severity_text", DataType::Utf8, true),
            Field::new("event_name", DataType::Utf8, true),
            Field::new(
                "body",
                DataType::Struct(Fields::from(vec![Field::new(
                    "string_value",
                    DataType::Utf8,
                    true,
                )])),
                true,
            ),
        ]));

        let mut time_builder = TimestampNanosecondBuilder::new();
        time_builder.append_value(TEST_TIMESTAMP as i64);
        time_builder.append_value(TEST_TIMESTAMP as i64);

        let mut obs_time_builder = TimestampNanosecondBuilder::new();
        obs_time_builder.append_value(TEST_OBSERVED_TIMESTAMP as i64);
        obs_time_builder.append_value(TEST_OBSERVED_TIMESTAMP as i64);

        let severity_array = Int32Array::from(vec![TEST_SEVERITY_NUMBER; 2]);
        let severity_text_array = StringArray::from(vec![Some(TEST_SEVERITY_TEXT); 2]);
        let event_name_array = StringArray::from(vec![Some("EventA"), Some("EventB")]);

        let string_value_array = StringArray::from(vec![Some(TEST_BODY); 2]);
        let body_array = StructArray::from(vec![(
            Arc::new(Field::new("string_value", DataType::Utf8, true)),
            Arc::new(string_value_array) as ArrayRef,
        )]);

        let otap_batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(time_builder.finish()),
                Arc::new(obs_time_builder.finish()),
                Arc::new(severity_array),
                Arc::new(severity_text_array),
                Arc::new(event_name_array),
                Arc::new(body_array),
            ],
        )
        .unwrap();

        let otap_result = otap_encoder
            .encode_log_batch(&otap_batch)
            .expect("OTAP encoding failed");

        // Both should produce 2 batches (one per event name)
        assert_eq!(
            otlp_result.len(),
            2,
            "OTLP should group by event name into 2 batches"
        );
        assert_eq!(
            otap_result.len(),
            2,
            "OTAP should group by event name into 2 batches"
        );

        // Collect event names from both results
        let mut otlp_event_names: Vec<_> =
            otlp_result.iter().map(|b| b.event_name.as_str()).collect();
        otlp_event_names.sort_unstable();

        let mut otap_event_names: Vec<_> =
            otap_result.iter().map(|b| b.event_name.as_str()).collect();
        otap_event_names.sort_unstable();

        assert_eq!(
            otlp_event_names, otap_event_names,
            "Both should produce same event names"
        );
        assert_eq!(otlp_event_names, vec!["EventA", "EventB"]);
    }

    #[test]
    fn test_compression_effectiveness() {
        let otlp_encoder = OtlpEncoder::new();
        let otap_encoder = OtapEncoder::new(TEST_METADATA.to_string());

        // Create a larger batch to better measure compression
        let batch_size = 100;

        let otlp_logs: Vec<LogRecord> = (0..batch_size)
            .map(|i| {
                let mut log = create_otlp_log_basic();
                log.time_unix_nano = TEST_TIMESTAMP + i * 1_000_000_000;
                log.body = Some(AnyValue {
                    value: Some(Value::StringValue(format!("Log message {}", i))),
                });
                log
            })
            .collect();

        // Create equivalent OTAP batch
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "time_unix_nano",
                DataType::Timestamp(TimeUnit::Nanosecond, None),
                true,
            ),
            Field::new(
                "observed_time_unix_nano",
                DataType::Timestamp(TimeUnit::Nanosecond, None),
                true,
            ),
            Field::new("severity_number", DataType::Int32, false),
            Field::new("severity_text", DataType::Utf8, true),
            Field::new("event_name", DataType::Utf8, true),
            Field::new(
                "body",
                DataType::Struct(Fields::from(vec![Field::new(
                    "string_value",
                    DataType::Utf8,
                    true,
                )])),
                true,
            ),
        ]));

        let mut time_builder = TimestampNanosecondBuilder::new();
        let mut obs_time_builder = TimestampNanosecondBuilder::new();
        let mut body_values = Vec::new();

        for i in 0..batch_size {
            time_builder.append_value((TEST_TIMESTAMP + i * 1_000_000_000) as i64);
            obs_time_builder.append_value(TEST_OBSERVED_TIMESTAMP as i64);
            body_values.push(Some(format!("Log message {}", i)));
        }

        let severity_array = Int32Array::from(vec![TEST_SEVERITY_NUMBER; batch_size as usize]);
        let severity_text_array = StringArray::from(vec![Some(TEST_SEVERITY_TEXT); batch_size as usize]);
        let event_name_array = StringArray::from(vec![Some(TEST_EVENT_NAME); batch_size as usize]);

        let string_value_array: StringArray = body_values.iter().map(|s| s.as_deref()).collect();
        let body_array = StructArray::from(vec![(
            Arc::new(Field::new("string_value", DataType::Utf8, true)),
            Arc::new(string_value_array) as ArrayRef,
        )]);

        let otap_batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(time_builder.finish()),
                Arc::new(obs_time_builder.finish()),
                Arc::new(severity_array),
                Arc::new(severity_text_array),
                Arc::new(event_name_array),
                Arc::new(body_array),
            ],
        )
        .unwrap();

        let otlp_result = otlp_encoder
            .encode_log_batch(otlp_logs.iter(), TEST_METADATA)
            .expect("OTLP encoding failed");
        let otap_result = otap_encoder
            .encode_log_batch(&otap_batch)
            .expect("OTAP encoding failed");

        let otlp_meta = EncodedMetadata::from_batch(&otlp_result[0]);
        let otap_meta = EncodedMetadata::from_batch(&otap_result[0]);

        let otlp_ratio =
            (otlp_meta.compressed_size as f64 / otlp_meta.uncompressed_size as f64) * 100.0;
        let otap_ratio =
            (otap_meta.compressed_size as f64 / otap_meta.uncompressed_size as f64) * 100.0;

        println!("Compression Effectiveness (100 logs):");
        println!(
            "  OTLP - Compressed: {} bytes, Uncompressed: {} bytes, Ratio: {:.2}%",
            otlp_meta.compressed_size, otlp_meta.uncompressed_size, otlp_ratio
        );
        println!(
            "  OTAP - Compressed: {} bytes, Uncompressed: {} bytes, Ratio: {:.2}%",
            otap_meta.compressed_size, otap_meta.uncompressed_size, otap_ratio
        );

        // Compression should be effective for both
        assert!(
            otlp_ratio < 100.0,
            "OTLP compression should reduce size"
        );
        assert!(
            otap_ratio < 100.0,
            "OTAP compression should reduce size"
        );
    }
}
