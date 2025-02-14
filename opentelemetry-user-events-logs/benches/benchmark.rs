use criterion::black_box;
use criterion::{criterion_group, criterion_main, Criterion};
use eventheader_dynamic::{Provider, ProviderOptions};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;

// System Configuration:
// Processor: AMD EPYC 7763 64-Core Processor
// CPU Cores: 8
// Logical Processors: 16
// Memory: 64 GB

// Benchmark Results (Grouped by Thread Count)

// Single-Threaded
// provider_enabled_only_single: 509.33 ps
// provider_find_set_only_single: 4.15 ns
// provider_find_set_enabled_single: 4.14 ns

// 2 Threads
// provider_enabled_only_concurrent_2threads: 523.28 ps
// provider_find_set_concurrent_2threads: 77.07 ns
// provider_find_set_enabled_concurrent_2threads: 119.42 ns

// 4 Threads
// provider_enabled_only_concurrent_4threads: 558.62 ps
// provider_find_set_concurrent_4threads: 148.91 ns
// provider_find_set_enabled_concurrent_4threads: 160.74 ns

// 8 Threads
// provider_enabled_only_concurrent_8threads: 660.90 ps
// provider_find_set_concurrent_8threads: 309.82 ns
// provider_find_set_enabled_concurrent_8threads: 533.21 ns

/// Benchmark `find_set`, `enabled`, and their combinations with no concurrent threads
fn benchmark_find_set_single(c: &mut Criterion) {
    // Setup the Provider
    let mut options = ProviderOptions::new();
    options = *options.group_name("testprovider");
    let mut provider = Provider::new("testprovider", &options);

    // Register some dummy events with specific levels and keywords
    let keyword = 0x01; // Example keyword
    let level = 4; // Example level (Informational)
    provider.register_set(eventheader::Level::Informational, keyword);

    // Benchmark the `enabled` method only
    if let Some(event_set) = provider.find_set(level.into(), keyword) {
        c.bench_function("provider_enabled_only_single", |b| {
            b.iter(|| {
                black_box(event_set.enabled()); // Only check `enabled`
            });
        });
    }

    // Benchmark `find_set` without `enabled` check
    c.bench_function("provider_find_set_only_single", |b| {
        b.iter(|| {
            if let Some(event_set) = provider.find_set(level.into(), keyword) {
                black_box(event_set); // No `enabled` check
            }
        });
    });

    // Benchmark `find_set` with `enabled` check
    c.bench_function("provider_find_set_enabled_single", |b| {
        b.iter(|| {
            if let Some(event_set) = provider.find_set(level.into(), keyword) {
                black_box(event_set.enabled()); // Check if the tracepoint is being listened to
            }
        });
    });
}

/// Benchmark `find_set`, `enabled`, and their combinations in a multi-threaded environment
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

        let provider = Arc::new(provider);

        // --- Test `enabled` only ---
        let stop_flag_enabled_only = Arc::new(AtomicBool::new(false));
        let mut enabled_only_handles = Vec::new();

        if let Some(event_set) = provider.find_set(level.into(), keyword) {
            // Start threads for `enabled` only
            for _ in 0..thread_count {
                let event_set_clone = event_set.clone();
                let stop_flag_clone = Arc::clone(&stop_flag_enabled_only);
                enabled_only_handles.push(thread::spawn(move || {
                    while !stop_flag_clone.load(Ordering::Relaxed) {
                        black_box(event_set_clone.enabled()); // Only check `enabled`
                    }
                }));
            }

            // Benchmark `enabled` only
            let enabled_only_benchmark_name =
                format!("provider_enabled_only_concurrent_{}threads", thread_count);
            c.bench_function(&enabled_only_benchmark_name, |b| {
                b.iter(|| {
                    black_box(event_set.enabled()); // Only check `enabled`
                });
            });
        }

        // Stop `enabled` only threads
        stop_flag_enabled_only.store(true, Ordering::Relaxed);
        for handle in enabled_only_handles {
            let _ = handle.join();
        }

        // --- Test `find_set` only ---
        let stop_flag_find_set = Arc::new(AtomicBool::new(false));
        let mut find_set_handles = Vec::new();

        // Start threads for `find_set`
        for _ in 0..thread_count {
            let provider_clone = Arc::clone(&provider);
            let stop_flag_clone = Arc::clone(&stop_flag_find_set);
            find_set_handles.push(thread::spawn(move || {
                while !stop_flag_clone.load(Ordering::Relaxed) {
                    if let Some(event_set) = provider_clone.find_set(level.into(), keyword) {
                        black_box(event_set); // No `enabled` check
                    }
                }
            }));
        }

        // Benchmark `find_set` only
        let find_set_benchmark_name =
            format!("provider_find_set_concurrent_{}threads", thread_count);
        c.bench_function(&find_set_benchmark_name, |b| {
            b.iter(|| {
                if let Some(event_set) = provider.find_set(level.into(), keyword) {
                    black_box(event_set); // No `enabled` check
                }
            });
        });

        // Stop `find_set` threads
        stop_flag_find_set.store(true, Ordering::Relaxed);
        for handle in find_set_handles {
            let _ = handle.join();
        }

        // --- Test `find_set` + `enabled` ---
        let stop_flag_enabled = Arc::new(AtomicBool::new(false));
        let mut enabled_handles = Vec::new();

        // Start threads for `find_set` + `enabled`
        for _ in 0..thread_count {
            let provider_clone = Arc::clone(&provider);
            let stop_flag_clone = Arc::clone(&stop_flag_enabled);
            enabled_handles.push(thread::spawn(move || {
                while !stop_flag_clone.load(Ordering::Relaxed) {
                    if let Some(event_set) = provider_clone.find_set(level.into(), keyword) {
                        black_box(event_set.enabled()); // Check if the tracepoint is being listened to
                    }
                }
            }));
        }

        // Benchmark `find_set` + `enabled`
        let enabled_benchmark_name = format!(
            "provider_find_set_enabled_concurrent_{}threads",
            thread_count
        );
        c.bench_function(&enabled_benchmark_name, |b| {
            b.iter(|| {
                if let Some(event_set) = provider.find_set(level.into(), keyword) {
                    black_box(event_set.enabled()); // Check if the tracepoint is being listened to
                }
            });
        });

        // Stop `find_set` + `enabled` threads
        stop_flag_enabled.store(true, Ordering::Relaxed);
        for handle in enabled_handles {
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
