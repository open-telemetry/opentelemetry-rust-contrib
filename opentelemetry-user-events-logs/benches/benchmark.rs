// Run as root user as access to `tracefs` is required, or ensure the current user has the necessary permissions.
// To run benchmarks with root privileges, execute:
//   sudo -E /home/<username>/.cargo/bin/cargo bench
// Replace <username> with your actual username.
//
// System Information:
// Processor: AMD EPYC 7763 64-Core Processor
// CPU Cores: 8
// Logical Processors: 16
// Memory: 64 GB
//  time:   [4.1429 ns 4.1514 ns 4.1621 ns]

use criterion::{criterion_group, criterion_main, Criterion};
use eventheader_dynamic::{Provider, ProviderOptions};

fn benchmark_find_set(c: &mut Criterion) {
    // Setup the Provider
    let mut options = ProviderOptions::new();
    options = *options.group_name("testprovider");
    let mut provider = Provider::new("testprovider", &options);

    // Register some dummy events with specific levels and keywords
    let keyword = 0x01; // Example keyword
    let mut level = 4; // Example level (Informational)
    provider.register_set(eventheader::Level::Informational, keyword);
    level = level.into();
    // Benchmark the `find_set` method
    c.bench_function("provider_find_set", |b| {
        b.iter(|| provider.find_set(level.into(), keyword));
    });
}

criterion_group!(benches, benchmark_find_set);
criterion_main!(benches);
