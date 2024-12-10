use criterion::{criterion_group, criterion_main, Criterion};
use eventheader_dynamic::{Provider, ProviderOptions};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;

/// Benchmark `find_set` with no concurrent threads
fn benchmark_find_set_single(c: &mut Criterion) {
    // Setup the Provider
    let mut options = ProviderOptions::new();
    options = *options.group_name("testprovider");
    let mut provider = Provider::new("testprovider", &options);

    // Register some dummy events with specific levels and keywords
    let keyword = 0x01; // Example keyword
    let level = 4; // Example level (Informational)
    provider.register_set(eventheader::Level::Informational, keyword);

    // Benchmark the `find_set` method with `enabled` check
    c.bench_function("provider_find_set_single", |b| {
        b.iter(|| {
            if let Some(event_set) = provider.find_set(level.into(), keyword) {
                event_set.enabled(); // Check if the tracepoint is being listened to
            }
        });
    });
}

/// Benchmark `find_set` with a parameterized number of concurrent threads
fn benchmark_find_set_concurrent(c: &mut Criterion) {
    let thread_counts = [2, 4, 8]; // Test with 2, 4, and 8 threads

    for &thread_count in &thread_counts {
        // Setup the Provider
        let mut options = ProviderOptions::new();
        options = *options.group_name("testprovider");
        let mut provider = Provider::new("testprovider", &options);

        // Register some dummy events with specific levels and keywords
        let keyword = 0x01; // Example keyword
        let level = 4; // Example level (Informational)
        provider.register_set(eventheader::Level::Informational, keyword);

        // Shared Provider and stop flag
        let provider = Arc::new(provider);
        let stop_flag = Arc::new(AtomicBool::new(false));

        // Spawn worker threads
        let mut worker_handles = Vec::new();
        for _ in 0..thread_count {
            let provider_clone = Arc::clone(&provider);
            let stop_flag_clone = Arc::clone(&stop_flag);
            worker_handles.push(thread::spawn(move || {
                while !stop_flag_clone.load(Ordering::Relaxed) {
                    if let Some(event_set) = provider_clone.find_set(level.into(), keyword) {
                        event_set.enabled(); // Check if tracepoint is being listened to
                    }
                }
            }));
        }

        // Dereference the `Arc` once before the benchmark to reduce overhead
        let provider_ref: &Provider = &provider;

        // Benchmark the `find_set` method with `enabled` check
        let benchmark_name = format!("provider_find_set_concurrent_{}threads", thread_count);
        c.bench_function(&benchmark_name, |b| {
            b.iter(|| {
                if let Some(event_set) = provider_ref.find_set(level.into(), keyword) {
                    event_set.enabled(); // Check if tracepoint is being listened to
                }
            });
        });

        // Signal worker threads to stop
        stop_flag.store(true, Ordering::Relaxed);

        // Wait for all worker threads to complete
        for handle in worker_handles {
            let _ = handle.join();
        }
    }
}

criterion_group!(
    benches,
    benchmark_find_set_single,
    benchmark_find_set_concurrent
);
criterion_main!(benches);
