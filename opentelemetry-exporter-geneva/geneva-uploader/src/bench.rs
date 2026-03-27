#[cfg(test)]
mod benchmarks {
    use crate::payload_encoder::lz4_chunked_compression::lz4_chunked_compression;
    use crate::payload_encoder::otlp_encoder::{MetadataFields, OtlpEncoder};
    use criterion::{BenchmarkId, Criterion, Throughput};
    use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
    use opentelemetry_proto::tonic::common::v1::any_value::Value;
    use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};
    use opentelemetry_proto::tonic::logs::v1::LogRecord;
    use otap_df_pdata::views::otlp::bytes::logs::RawLogsData;
    use prost::Message as _;
    use rand::{rngs::StdRng, Rng, SeedableRng};
    use std::hint::black_box;

    fn setup_test_data(size: usize) -> Vec<u8> {
        let mut data = vec![0u8; size];
        let mut rng = StdRng::seed_from_u64(42);
        rng.fill(&mut data[..]);
        data
    }

    fn make_metadata(namespace: &str) -> MetadataFields {
        MetadataFields::new(
            "BenchmarkEnv".to_string(),
            "Ver1v0".to_string(),
            "BenchmarkTenant".to_string(),
            "BenchmarkRole".to_string(),
            "BenchmarkRoleInstance".to_string(),
            namespace.to_string(),
            "Ver1v0".to_string(),
        )
    }

    fn create_basic_log_record(timestamp: u64) -> LogRecord {
        LogRecord {
            observed_time_unix_nano: timestamp,
            severity_number: 9,
            severity_text: "INFO".to_string(),
            event_name: "BasicEvent".to_string(),
            body: Some(AnyValue {
                value: Some(Value::StringValue("Basic log message".to_string())),
            }),
            ..Default::default()
        }
    }

    fn create_log_with_attributes(
        timestamp: u64,
        num_attributes: usize,
        rng: &mut StdRng,
    ) -> LogRecord {
        let mut log = LogRecord {
            observed_time_unix_nano: timestamp,
            severity_number: 9,
            severity_text: "INFO".to_string(),
            event_name: format!("Event_{num_attributes}Attrs"),
            body: Some(AnyValue {
                value: Some(Value::StringValue("Log with attributes".to_string())),
            }),
            ..Default::default()
        };

        for i in 0..num_attributes {
            let key = format!("attr_{i}");
            let value = match i % 4 {
                0 => AnyValue {
                    value: Some(Value::StringValue(format!(
                        "string_value_{}",
                        rng.random::<u32>()
                    ))),
                },
                1 => AnyValue {
                    value: Some(Value::IntValue(rng.random_range(0..1000))),
                },
                2 => AnyValue {
                    value: Some(Value::DoubleValue(rng.random_range(0.0..100.0))),
                },
                _ => AnyValue {
                    value: Some(Value::BoolValue(rng.random::<f64>() < 0.5)),
                },
            };

            log.attributes.push(KeyValue {
                key,
                value: Some(value),
            });
        }

        log
    }

    fn encode_logs_request_bytes(logs: Vec<LogRecord>) -> Vec<u8> {
        ExportLogsServiceRequest {
            resource_logs: vec![opentelemetry_proto::tonic::logs::v1::ResourceLogs {
                scope_logs: vec![opentelemetry_proto::tonic::logs::v1::ScopeLogs {
                    log_records: logs,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
        .encode_to_vec()
    }

    /*
        - Criterion benchmarks (lz4_chunked_compression, lz4_flex backend):

            | Input size  | Median time        | Throughput         |
            |-------------|--------------------|--------------------|
            | 1 byte      | ~533 ns            | ~1.6 MiB/s         |
            | 1 KiB       | ~597 ns           | ~1.5 GiB/s        |
            | 1 MiB       | ~42 us           | ~22.1 GiB/s         |
        - No significant regressions or improvements detected.
        - Machine: Apple M4 Pro, 24 GB,  Total Number of Cores:	14 (10 performance and 4 efficiency)
    */
    #[test]
    #[ignore = "benchmark on crate private, ignored by default during normal test runs"]
    /// To run: $cargo test --release lz4_benchmark -- --nocapture --ignored
    fn lz4_benchmark() {
        let mut criterion = Criterion::default();

        let mut group = criterion.benchmark_group("lz4_compression");

        for size in [1, 1024, 1024 * 1024] {
            let data = setup_test_data(size);

            group.throughput(Throughput::Bytes(size as u64));
            group.bench_with_input(BenchmarkId::new("lz4_flex", size), &data, |b, data| {
                b.iter(|| black_box(lz4_chunked_compression(data).unwrap()));
            });
        }

        group.finish();
        criterion.final_summary();
    }

    /*
        - Criterion benchmark (encode_logs_from_view, RawLogsData-backed):
        - Mirrors the earlier log-encoding workload shapes using the current
          view-based entrypoint.
    */
    #[test]
    #[ignore = "benchmark on crate private, ignored by default during normal test runs"]
    /// To run: $cargo test --release encode_logs_from_view_benchmark -- --nocapture --ignored
    fn encode_logs_from_view_benchmark() {
        let mut criterion = Criterion::default();
        let encoder = OtlpEncoder::new();
        let metadata = make_metadata("benchmark");

        let mut group = criterion.benchmark_group("encode_logs_from_view_attributes");
        for num_attrs in [0usize, 4, 8, 16] {
            let request_bytes = {
                let mut rng = StdRng::seed_from_u64(42);
                let logs: Vec<LogRecord> = (0..10)
                    .map(|i| {
                        if num_attrs == 0 {
                            create_basic_log_record(1_700_000_000_000_000_000 + i)
                        } else {
                            create_log_with_attributes(
                                1_700_000_000_000_000_000 + i,
                                num_attrs,
                                &mut rng,
                            )
                        }
                    })
                    .collect();
                encode_logs_request_bytes(logs)
            };

            group.throughput(Throughput::Elements(num_attrs as u64));
            group.bench_with_input(
                BenchmarkId::new("attributes", num_attrs),
                &request_bytes,
                |b, request_bytes| {
                    b.iter(|| {
                        let view = RawLogsData::new(black_box(request_bytes.as_slice()));
                        let res = encoder
                            .encode_logs_from_view(black_box(&view), black_box(&metadata))
                            .unwrap();
                        black_box(res);
                    });
                },
            );
        }
        group.finish();

        let mut group = criterion.benchmark_group("encode_logs_from_view_sizes");
        for batch_size in [1usize, 10, 100, 1000] {
            let request_bytes = {
                let mut rng = StdRng::seed_from_u64(42);
                let logs: Vec<LogRecord> = (0..batch_size)
                    .map(|i| {
                        create_log_with_attributes(
                            1_700_000_000_000_000_000 + i as u64,
                            4,
                            &mut rng,
                        )
                    })
                    .collect();
                encode_logs_request_bytes(logs)
            };

            group.bench_with_input(
                BenchmarkId::new("batch_size", batch_size),
                &request_bytes,
                |b, request_bytes| {
                    b.iter(|| {
                        let view = RawLogsData::new(black_box(request_bytes.as_slice()));
                        let res = encoder
                            .encode_logs_from_view(black_box(&view), black_box(&metadata))
                            .unwrap();
                        black_box(res);
                    });
                },
            );
        }
        group.finish();

        let mut group = criterion.benchmark_group("encode_logs_from_view_mixed_event_names");
        let mixed_request_bytes = {
            let mut rng = StdRng::seed_from_u64(42);
            let event_names = ["EventA", "EventB", "EventC"];
            let logs: Vec<LogRecord> = (0..100)
                .map(|i| {
                    let mut log =
                        create_log_with_attributes(1_700_000_000_000_000_000 + i, 4, &mut rng);
                    log.event_name = event_names[i as usize % event_names.len()].to_string();
                    log
                })
                .collect();
            encode_logs_request_bytes(logs)
        };
        group.bench_function("mixed_event_names", |b| {
            b.iter(|| {
                let view = RawLogsData::new(black_box(mixed_request_bytes.as_slice()));
                let res = encoder
                    .encode_logs_from_view(black_box(&view), black_box(&metadata))
                    .unwrap();
                black_box(res);
            });
        });
        group.finish();

        criterion.final_summary();
    }
}
