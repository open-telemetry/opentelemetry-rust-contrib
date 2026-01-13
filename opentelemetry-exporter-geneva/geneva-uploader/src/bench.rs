#[cfg(test)]
mod benchmarks {
    use crate::payload_encoder::lz4_chunked_compression::lz4_chunked_compression;
    use crate::payload_encoder::otlp_encoder::{MetadataFields, OtlpEncoder};
    use criterion::{BenchmarkId, Criterion, Throughput};
    use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};
    use opentelemetry_proto::tonic::logs::v1::LogRecord;
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

    // Helper functions for encode_log_batch benchmark
    fn create_basic_log_record(timestamp: u64) -> LogRecord {
        LogRecord {
            observed_time_unix_nano: timestamp,
            severity_number: 9,
            severity_text: "INFO".to_string(),
            event_name: "BasicEvent".to_string(),
            body: Some(AnyValue {
                value: Some(
                    opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                        "Basic log message".to_string(),
                    ),
                ),
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
                value: Some(
                    opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                        "Log with attributes".to_string(),
                    ),
                ),
            }),
            ..Default::default()
        };

        // Add attributes with different types
        for i in 0..num_attributes {
            let key = format!("attr_{i}");
            let value = match i % 4 {
                0 => AnyValue {
                    value: Some(
                        opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                            format!("string_value_{}", rng.random::<u32>()),
                        ),
                    ),
                },
                1 => AnyValue {
                    value: Some(
                        opentelemetry_proto::tonic::common::v1::any_value::Value::IntValue(
                            rng.random_range(0..1000),
                        ),
                    ),
                },
                2 => AnyValue {
                    value: Some(
                        opentelemetry_proto::tonic::common::v1::any_value::Value::DoubleValue(
                            rng.random_range(0.0..100.0),
                        ),
                    ),
                },
                _ => AnyValue {
                    value: Some(
                        opentelemetry_proto::tonic::common::v1::any_value::Value::BoolValue(
                            rng.random::<f64>() < 0.5,
                        ),
                    ),
                },
            };

            log.attributes.push(KeyValue {
                key,
                value: Some(value),
            });
        }

        log
    }

    /*
    - Criterion benchmarks (encode_log_batch, OtlpEncoder):
        Results: (WSL2 Linux, 62 GiB RAM, 8-core/16-thread Intel CPU, kernel 6.6.36.3)
        - Attribute count scaling (10 log records each):
            - 0 attrs:   ~10.3 µs/op
            - 4 attrs:   ~15.7 µs/op
            - 8 attrs:   ~24.3 µs/op
            - 16 attrs:  ~44.1 µs/op

        - Batch size scaling (each log with 4 attributes, all same schema):
            - 1 log:     ~1.63 µs/op
            - 10 logs:   ~16.4 µs/op
            - 100 logs:  ~161 µs/op
            - 1000 logs: ~1.59 ms/op

        - Mixed event names (100 logs, 3 different event names, 4 attributes each):
            - ~168 µs/op
    */
    #[test]
    #[ignore = "benchmark on crate private, ignored by default during normal test runs"]
    // To run: $cargo test --release encode_log_batch_benchmark -- --nocapture --ignored
    fn encode_log_batch_benchmark() {
        let mut criterion = Criterion::default();
        let encoder = OtlpEncoder::new();
        let metadata = make_metadata("benchmark");

        // Benchmark 1: Different numbers of attributes
        let mut group = criterion.benchmark_group("encode_log_batch_attributes");
        for num_attrs in [0, 4, 8, 16].iter() {
            group.throughput(Throughput::Elements(*num_attrs as u64));
            group.bench_with_input(
                BenchmarkId::new("attributes", num_attrs),
                num_attrs,
                |b, &num_attrs| {
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

                    b.iter(|| {
                        let res = encoder
                            .encode_log_batch(black_box(logs.iter()), black_box(&metadata))
                            .unwrap();
                        black_box(res); // double sure the return value is generated
                    });
                },
            );
        }
        group.finish();

        // Benchmark 2: Different batch sizes
        let mut group = criterion.benchmark_group("encode_log_batch_sizes");
        for batch_size in [1, 10, 100, 1000].iter() {
            group.bench_with_input(
                BenchmarkId::new("batch_size", batch_size),
                batch_size,
                |b, &batch_size| {
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

                    b.iter(|| {
                        let res = black_box(
                            encoder
                                .encode_log_batch(black_box(logs.iter()), black_box(&metadata))
                                .unwrap(),
                        );
                        black_box(res); // double sure the return value is generated
                    });
                },
            );
        }
        group.finish();

        // Benchmark 3: Mixed event names
        let mut group = criterion.benchmark_group("encode_log_batch_mixed_event_names");
        group.bench_function("mixed_event_names", |b| {
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

            b.iter(|| {
                let res = black_box(
                    encoder
                        .encode_log_batch(black_box(logs.iter()), black_box(&metadata))
                        .unwrap(),
                );
                black_box(res);
            });
        });
        group.finish();

        criterion.final_summary();
    }
}
