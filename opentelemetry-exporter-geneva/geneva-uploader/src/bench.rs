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
            | 1 byte      | ~711 ns            | ~1.3 MiB/s         |
            | 1 KiB       | ~31.8 us           | ~30.6 MiB/s        |
            | 1 MiB       | ~3.44 ms           | ~291 MiB/s         |
        - No significant regressions or improvements detected.
        - Machine: WSL2, Linux 6.6.36, AMD EPYC 7763 64-core (8 cores visible), 62GiB RAM, 16GiB swap.
    */
    #[test]
    fn lz4_benchmark() {
        let mut criterion = Criterion::default()
            .sample_size(100)
            .measurement_time(std::time::Duration::from_secs(5));

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
