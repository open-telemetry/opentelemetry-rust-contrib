#[cfg(test)]
mod benchmarks {
    use crate::payload_encoder::lz4_chunked_compression::lz4_chunked_compression;
    use criterion::{BenchmarkId, Criterion, Throughput};
    use rand::{rngs::StdRng, Rng, SeedableRng};
    use std::hint::black_box;

    fn setup_test_data(size: usize) -> Vec<u8> {
        let mut data = vec![0u8; size];
        let mut rng = StdRng::seed_from_u64(42);
        rng.fill(&mut data[..]);
        data
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
}
