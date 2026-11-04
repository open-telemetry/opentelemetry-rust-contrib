/*
    Serialization optimization comparison: 2026-11-04, rustc 1.99.0.
    Machine inventory captured with this run:
    AMD EPYC 7763; 8 guest cores / 16 logical CPUs; 63.95 GiB RAM;
    Microsoft Virtual Machine; Windows 11 Enterprise x64 10.0.26300.
    One synchronous producer thread, no affinity override (process mask 0xffff).

    Baseline: 97d8baa. Optimized: this implementation, before results comments.
    Two processes per variant, ordered baseline/optimized/optimized/baseline.
    Criterion 0.8.2: 100 samples, 1 s warm-up, 2 s measurement, 100,000 resamples.
    Arithmetic means of the two per-process point estimates, in us/span:

    | Attributes | Serialization before | After | Enabled export before | After |
    |------------|----------------------|-------|-----------------------|-------|
    | 0          | 0.594                | 0.392 | 1.537                 | 1.413 |
    | 3 mixed    | 0.730                | 0.447 | 1.675                 | 1.435 |
    | 6 strings  | 0.797                | 0.502 | 2.016                 | 1.458 |

    Serialization decreased 34.0%, 38.7%, 37.0%; enabled-export point estimates
    decreased 8.0%, 14.4%, 27.7%. Export is noisier: the 0-attribute direction
    reversed in the second pair, so its improvement is not established.

    Both stages use the same prebuilt SpanData/resource and production encoder,
    with warm reusable buffers. Export also includes the enabled check, TLS
    builder borrow, and real synchronous ETW write. Neither includes SDK span
    creation/context/cleanup. Serialization has no listener dependency.
    This is not a write-only measurement or a full SDK lifecycle measurement.

    Build: cargo bench -p opentelemetry-etw-traces --bench traces --features bench-internals --no-run
    Run the resulting EXE with OTEL_ETW_BENCH_MODE=enabled and an ETW listener:
      & $exe 'Serialization|/Export/' --bench --warm-up-time 1 --measurement-time 2 --noplot
    File-mode capture: keyword 0x1, level 5, 64 KiB buffers, min 8192 / max 16384;
    no PID filter or clock override. All 26,789,462 captured provider events
    (including warm-up) matched baseline metadata/payload bytes; zero reported loss.
    Longer optimized attempts lost events and were discarded. Both variants were
    rerun with these identical settings; Criterion defaults in code are unchanged.
    See README.md for per-run intervals and historical pre-optimization results.
*/

/*
    Preliminary ETW benchmark results: 2026-11-03, rustc 1.99.0.
    Machine inventory queried after the runs, on 2026-11-03:
    CPU: AMD EPYC 7763 64-Core Processor; guest reports 8 cores / 16 logical CPUs.
    RAM: 63.95 GiB (approximately 64 GiB).
    Machine: Microsoft Virtual Machine.
    OS: Windows 11 Enterprise x64, version 10.0.26300 (build 26300).

    Criterion point estimates: 10 samples, 100 ms warm-up, 200 ms measurement.
    Enabled capture: file-mode ETL, 64 KiB buffers, min 128 / max 512,
    keyword 0x1, level 5; no PID filter or CPU-affinity override.

    | Benchmark    | ETW enabled | ETW disabled |
    |--------------|-------------|--------------|
    | SimpleSpan   | 3.02 us     | 379 ns       |
    | ScopedSpan/0 | 1.83 us     | 439 ns       |
    | ScopedSpan/3 | 2.63 us     | 502 ns       |

    See README.md for exact commands and provenance limitations. These short
    measurements are not regression thresholds or a controlled comparison with
    user-events/Linux; the hardware inventory was not captured during the runs.
*/

#[cfg(windows)]
mod windows {
    use criterion::{BenchmarkId, Criterion};
    use opentelemetry::trace::{Span, SpanKind, Status, TraceContextExt, Tracer, TracerProvider};
    use opentelemetry::{Context, KeyValue};
    use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};
    use std::env::VarError;
    use std::sync::Once;
    use std::time::{Duration, Instant};
    use tracelogging_dynamic::{Level, Provider};

    const PROVIDER_NAME: &str = "OpenTelemetryEtwTraceBenchmark";

    fn assert_listener_state(probe: &Provider, expected: bool) {
        assert_eq!(
            probe.enabled(Level::Verbose, 1),
            expected,
            "ETW listener state does not match OTEL_ETW_BENCH_MODE; \
             see the crate README for capture instructions"
        );
    }

    fn enabled_mode() -> bool {
        match std::env::var("OTEL_ETW_BENCH_MODE").as_deref() {
            Ok("enabled") => true,
            Ok("disabled") | Err(VarError::NotPresent) => false,
            _ => panic!("OTEL_ETW_BENCH_MODE must be 'enabled' or 'disabled'"),
        }
    }

    pub fn benchmark(c: &mut Criterion) {
        let enabled = enabled_mode();
        let mode = if enabled { "enabled" } else { "disabled" };
        let provider = SdkTracerProvider::builder()
            .with_sampler(Sampler::AlwaysOn)
            .with_resource(
                opentelemetry_sdk::Resource::builder_empty()
                    .with_service_name("etw-trace-benchmark")
                    .build(),
            )
            .with_span_processor(
                opentelemetry_etw_traces::Processor::builder(PROVIDER_NAME)
                    .build()
                    .expect("Valid benchmark provider name"),
            )
            .build();
        let tracer = provider.tracer("etw-trace-benchmark");
        let probe = Box::pin(Provider::new(PROVIDER_NAME, &Provider::options()));
        // SAFETY: the probe stays pinned until dropped, which unregisters its ETW callback.
        assert_eq!(unsafe { probe.as_ref().register() }, 0);
        let listener_setup = Once::new();
        let setup_listener = || {
            listener_setup.call_once(|| {
                if enabled {
                    let deadline = Instant::now() + Duration::from_secs(5);
                    while !probe.enabled(Level::Verbose, 1) && Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            });
            assert_listener_state(&probe, enabled);
        };

        let mut group = c.benchmark_group(format!("ETW/{mode}"));
        group.bench_function("SimpleSpan", |b| {
            setup_listener();
            b.iter(|| {
                let mut span = tracer
                    .span_builder("my-span-name")
                    .with_attributes([
                        KeyValue::new("key1", "value1"),
                        KeyValue::new("key2", "value2"),
                        KeyValue::new("key3", "value3"),
                        KeyValue::new("key4", "value4"),
                        KeyValue::new("key5", "value5"),
                        KeyValue::new("key6", "value6"),
                    ])
                    .start(&tracer);
                span.end();
            });
            assert_listener_state(&probe, enabled);
        });

        for count in [0, 3] {
            let attributes = if count == 0 {
                vec![]
            } else {
                vec![
                    KeyValue::new("string_key", "benchmark-value"),
                    KeyValue::new("integer_key", 42_i64),
                    KeyValue::new("bool_key", true),
                ]
            };
            group.bench_with_input(
                BenchmarkId::new("ScopedSpan", count),
                &attributes,
                |b, attributes| {
                    setup_listener();
                    b.iter(|| {
                        let span = tracer
                            .span_builder("BenchmarkOperation")
                            .with_kind(SpanKind::Server)
                            .with_attributes(attributes.iter().cloned())
                            .start(&tracer);
                        let context = Context::current_with_span(span);
                        let scope = context.clone().attach();
                        context.span().set_status(Status::Ok);
                        context.span().end();
                        drop(scope);
                        drop(context);
                    });
                    assert_listener_state(&probe, enabled);
                },
            );
        }
        group.finish();
        provider.shutdown().expect("ETW provider shutdown");
    }

    #[cfg(feature = "bench-internals")]
    pub fn stages(c: &mut Criterion) {
        use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
        use opentelemetry_etw_traces::benchmark::ExportBenchmark;
        use opentelemetry_sdk::trace::SpanData;
        use std::hint::black_box;
        use std::time::UNIX_EPOCH;
        use tracelogging_dynamic::EventBuilder;

        let resource = opentelemetry_sdk::Resource::builder_empty()
            .with_service_name("etw-trace-benchmark")
            .build();
        let exporter = ExportBenchmark::new(PROVIDER_NAME, &resource);
        let enabled = enabled_mode();
        let mode = if enabled { "enabled" } else { "disabled" };
        let listener_setup = Once::new();

        for count in [0, 3, 6] {
            let attributes = match count {
                0 => vec![],
                3 => vec![
                    KeyValue::new("string_key", "benchmark-value"),
                    KeyValue::new("integer_key", 42_i64),
                    KeyValue::new("bool_key", true),
                ],
                _ => (1..=6)
                    .map(|i| KeyValue::new(format!("key{i}"), format!("value{i}")))
                    .collect(),
            };
            let span = SpanData {
                span_context: SpanContext::new(
                    TraceId::from_bytes([0x12; 16]),
                    SpanId::from_bytes([0x34; 8]),
                    TraceFlags::SAMPLED,
                    false,
                    TraceState::default(),
                ),
                parent_span_id: SpanId::INVALID,
                parent_span_is_remote: false,
                span_kind: SpanKind::Server,
                name: "BenchmarkOperation".into(),
                start_time: UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789),
                end_time: UNIX_EPOCH + Duration::new(1_700_000_001, 123_456_789),
                attributes,
                dropped_attributes_count: 0,
                events: Default::default(),
                links: Default::default(),
                status: Status::Ok,
                instrumentation_scope: Default::default(),
            };

            c.bench_with_input(
                BenchmarkId::new("ETW/Serialization", count),
                &span,
                |b, span| {
                    let mut event = EventBuilder::new();
                    exporter.serialize(&mut event, span);
                    b.iter(|| {
                        exporter.serialize(black_box(&mut event), black_box(span));
                        black_box(&event);
                    });
                },
            );
            c.bench_with_input(
                BenchmarkId::new(format!("ETW/{mode}/Export"), count),
                &span,
                |b, span| {
                    listener_setup.call_once(|| {
                        if enabled {
                            let deadline = Instant::now() + Duration::from_secs(5);
                            while !exporter.enabled() && Instant::now() < deadline {
                                std::thread::sleep(Duration::from_millis(10));
                            }
                        }
                    });
                    assert_eq!(exporter.enabled(), enabled, "ETW listener state mismatch");
                    b.iter(|| exporter.export(black_box(span)));
                    assert_eq!(exporter.enabled(), enabled, "ETW listener state mismatch");
                },
            );
        }
    }
}

#[cfg(all(windows, not(feature = "bench-internals")))]
criterion::criterion_group!(benches, windows::benchmark);
#[cfg(all(windows, feature = "bench-internals"))]
criterion::criterion_group!(benches, windows::benchmark, windows::stages);
#[cfg(windows)]
criterion::criterion_main!(benches);

#[cfg(not(windows))]
fn main() {
    eprintln!("The ETW trace benchmarks require Windows; no measurements were taken.");
}
