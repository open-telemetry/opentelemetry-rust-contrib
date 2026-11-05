/*
    Idiomatic SDK / end-to-end lifecycle results: 2026-11-04, rustc 1.99.0.
    Mean of two per-process Criterion slope estimates, us/span:

    | Case        | SDK-only | Nested SDK + serialize + ETW | Flat Geneva SDK + serialize + ETW |
    |-------------|----------|------------------------------|------------------------------------|
    | 0           | 0.343    | 2.111                        | 1.830                              |
    | 3 mixed     | 0.412    | 2.191                        | 2.309                              |
    | 6 strings   | 0.446    | 2.333                        | 2.209                              |
    | ParentError | 0.411    | 2.378                        | 2.290                              |

    Shared idiomatic lifecycle: build/start a sampled Server span, clone attributes
    into it, attach current context with an RAII guard, set status, explicitly end,
    then detach and drop context/span on scope exit. Fresh SDK IDs/times are included.
    Fixed keys/string values are literals. Inputs/provider/parent setup are untimed.
    ParentError is a child of a prebuilt remote parent and has 3 mixed attributes.
    SDK-only uses AlwaysOn with NO processor; recording is checked outside timing.
    It excludes export-data handoff and is NOT additive with exporter-only timings.
    EndToEnd measures the actual synchronous ETW processor and write, not a sum.

    Two fresh processes; each runs SDK-only, nested EndToEnd, then Geneva EndToEnd.
    Criterion 0.8.2: 100 samples, 1 s warm-up, 2 s measurement, 100,000 resamples.
    One unpinned producer (mask 0xffff). Microsoft VM, EPYC 7763, 8 guest cores /
    16 logical CPUs, 63.95 GiB RAM, Windows 11 Enterprise x64 10.0.26300.
    ETW file capture: 64 KiB buffers, min 8192 / max 16384, keyword 1, level 5;
    no PID/clock override. Validated 22,528,592 measured-capture events, zero loss.
    SDK-only smoke emitted zero events; EndToEnd smoke emitted exactly eight.
    Dynamic IDs/timestamps were validated, not compared byte-for-byte to fixed inputs.
    End-to-end estimates are noisy; do not infer a stable layout ranking.
    An earlier heap-owned six-string input pilot is excluded from these results.

    With an ETW listener and OTEL_ETW_BENCH_MODE=enabled:
      & $exe 'RecordingLifecycle|/EndToEnd/' --bench --warm-up-time 1 --measurement-time 2 --noplot
    Build with --features bench-internals,experimental-geneva-mapping.
    See README.md for the exact measurement boundaries and provenance.
*/

/*
    Dual-layout serialization/export comparison: 2026-11-04, rustc 1.99.0.
    All PR #877 runtime optimizations cover both layouts, including env_time,
    envelope IDs, parent IDs, borrowed error descriptions, and extended years.
    The attribute-classification optimization is also retained in both layouts.

    Baseline: upstream 3bfc183 plus the same shared-encoder benchmark adapter.
    Optimized: this branch's working tree based on 47b62c7, before result comments.
    Same benchmark source/dependency tree; separate build directories.
    Mean of two per-process Criterion slope estimates, us/span:

    | Layout | Case        | Serialize before | After | Enabled export before | After |
    |--------|-------------|------------------|-------|-----------------------|-------|
    | nested | 0           | 0.586            | 0.415 | 1.703                 | 1.366 |
    | nested | 3           | 0.720            | 0.467 | 1.914                 | 1.496 |
    | nested | 6           | 0.806            | 0.512 | 1.920                 | 1.554 |
    | nested | ParentError | 0.937            | 0.504 | 2.173                 | 1.497 |
    | geneva | 0           | 0.568            | 0.382 | 1.798                 | 1.326 |
    | geneva | 3           | 0.678            | 0.425 | 2.004                 | 1.308 |
    | geneva | 6           | 0.750            | 0.480 | 2.000                 | 1.492 |
    | geneva | ParentError | 0.903            | 0.463 | 2.168                 | 1.434 |

    Enabled export INCLUDES serialization; neither stage includes SDK lifecycle.
    ParentError has 3 mixed attributes, a nonzero parent ID, and an error description.
    Two processes per variant, baseline/optimized/optimized/baseline order.
    Criterion 0.8.2: 100 samples, 1 s warm-up, 2 s measurement, 100,000 resamples.
    One synchronous producer, unpinned (mask 0xffff). Microsoft VM, EPYC 7763,
    8 guest cores / 16 logical CPUs, 63.95 GiB RAM, Windows 11 x64 10.0.26300.
    64 KiB ETW buffers, min 8192 / max 16384, keyword 1, level 5; no PID/clock override.
    All 66,854,032 measured-capture events matched baseline metadata/payload bytes;
    zero reported loss. Warm-up events included; smoke captures excluded.
    Export estimates are noisy VM/file-capture observations, not regression thresholds.

    Build: cargo bench -p opentelemetry-etw-traces --bench traces --features bench-internals,experimental-geneva-mapping --no-run
    With an ETW listener and OTEL_ETW_BENCH_MODE=enabled:
      & $exe 'Serialization|/Export/' --bench --warm-up-time 1 --measurement-time 2 --noplot
    See README.md for coverage, provenance, and interpretation.
*/

/*
    Historical nested-only optimization comparison at 47b62c7: 2026-11-04, rustc 1.99.0.
    Machine inventory captured with this run:
    AMD EPYC 7763; 8 guest cores / 16 logical CPUs; 63.95 GiB RAM;
    Microsoft Virtual Machine; Windows 11 Enterprise x64 10.0.26300.
    One synchronous producer thread, no affinity override (process mask 0xffff).

    Baseline: 97d8baa. Optimized: 47b62c7, before results comments.
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

    #[cfg(feature = "bench-internals")]
    const CASES: [(&str, usize); 4] = [("0", 0), ("3", 3), ("6", 6), ("ParentError", 3)];

    #[cfg(feature = "bench-internals")]
    fn attributes(count: usize) -> Vec<KeyValue> {
        match count {
            0 => vec![],
            3 => vec![
                KeyValue::new("string_key", "benchmark-value"),
                KeyValue::new("integer_key", 42_i64),
                KeyValue::new("bool_key", true),
            ],
            _ => vec![
                KeyValue::new("key1", "value1"),
                KeyValue::new("key2", "value2"),
                KeyValue::new("key3", "value3"),
                KeyValue::new("key4", "value4"),
                KeyValue::new("key5", "value5"),
                KeyValue::new("key6", "value6"),
            ],
        }
    }

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
        stage(c, false);
        #[cfg(feature = "experimental-geneva-mapping")]
        stage(c, true);
    }

    #[cfg(feature = "bench-internals")]
    pub fn lifecycles(c: &mut Criterion) {
        lifecycle_group(c, None);
        lifecycle_group(c, Some(false));
        #[cfg(feature = "experimental-geneva-mapping")]
        lifecycle_group(c, Some(true));
    }

    #[cfg(feature = "bench-internals")]
    fn record_span(
        tracer: &opentelemetry_sdk::trace::SdkTracer,
        attributes: &[KeyValue],
        error: bool,
    ) {
        let span = tracer
            .span_builder("BenchmarkOperation")
            .with_kind(SpanKind::Server)
            .with_attributes(attributes.iter().cloned())
            .start(tracer);
        let context = Context::current_with_span(span);
        let _guard = context.clone().attach();
        context.span().set_status(if error {
            Status::error("request failed")
        } else {
            Status::Ok
        });
        context.span().end();
    }

    #[cfg(feature = "bench-internals")]
    fn lifecycle_group(c: &mut Criterion, geneva: Option<bool>) {
        use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};

        let mut builder = SdkTracerProvider::builder()
            .with_sampler(Sampler::AlwaysOn)
            .with_resource(
                opentelemetry_sdk::Resource::builder_empty()
                    .with_service_name("etw-trace-benchmark")
                    .build(),
            );
        if geneva.is_some() {
            let processor = opentelemetry_etw_traces::Processor::builder(PROVIDER_NAME);
            #[cfg(feature = "experimental-geneva-mapping")]
            let processor = processor.with_geneva_mapping(geneva.unwrap_or(false));
            builder = builder
                .with_span_processor(processor.build().expect("Valid benchmark provider name"));
        }
        let provider = builder.build();
        let tracer = provider.tracer("etw-trace-benchmark");
        let enabled = enabled_mode();
        let mode = if enabled { "enabled" } else { "disabled" };
        let name = match geneva {
            None => "SDK/RecordingLifecycle".to_owned(),
            Some(false) => format!("ETW/nested/{mode}/EndToEnd"),
            Some(true) => format!("ETW/geneva/{mode}/EndToEnd"),
        };
        let probe = geneva.map(|_| {
            let probe = Box::pin(Provider::new(PROVIDER_NAME, &Provider::options()));
            // SAFETY: the probe stays pinned until drop unregisters its ETW callback.
            assert_eq!(unsafe { probe.as_ref().register() }, 0);
            probe
        });
        if geneva.is_none() {
            let span = tracer.start("recording-check");
            assert!(
                span.is_recording(),
                "SDK-only must not measure a no-op tracer"
            );
        }
        let listener_setup = Once::new();
        let mut group = c.benchmark_group(name);
        for (case, count) in CASES {
            let attributes = attributes(count);
            let error = case == "ParentError";
            let parent = if error {
                Context::new().with_remote_span_context(SpanContext::new(
                    TraceId::from_bytes([0x12; 16]),
                    SpanId::from_bytes([0x0a; 8]),
                    TraceFlags::SAMPLED,
                    true,
                    TraceState::default(),
                ))
            } else {
                Context::new()
            };
            group.bench_with_input(
                BenchmarkId::from_parameter(case),
                &attributes,
                |b, attributes| {
                    if let Some(probe) = &probe {
                        listener_setup.call_once(|| {
                            if enabled {
                                let deadline = Instant::now() + Duration::from_secs(5);
                                while !probe.enabled(Level::Verbose, 1) && Instant::now() < deadline
                                {
                                    std::thread::sleep(Duration::from_millis(10));
                                }
                            }
                        });
                        assert_listener_state(probe, enabled);
                    }
                    let _parent_guard = parent.clone().attach();
                    b.iter(|| record_span(&tracer, attributes, error));
                    assert_eq!(
                        Context::current().span().span_context(),
                        parent.span().span_context(),
                        "Lifecycle did not restore the parent context"
                    );
                    if let Some(probe) = &probe {
                        assert_listener_state(probe, enabled);
                    }
                },
            );
        }
        group.finish();
        provider.shutdown().expect("SDK provider shutdown");
    }

    #[cfg(feature = "bench-internals")]
    fn stage(c: &mut Criterion, geneva: bool) {
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
        #[cfg(feature = "experimental-geneva-mapping")]
        let exporter = exporter.with_geneva_mapping(geneva);
        let layout = if geneva { "geneva" } else { "nested" };
        let enabled = enabled_mode();
        let mode = if enabled { "enabled" } else { "disabled" };
        let listener_setup = Once::new();

        for (case, count) in CASES {
            let attributes = attributes(count);
            let mut span = SpanData {
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
            if case == "ParentError" {
                span.parent_span_id = SpanId::from_bytes([0x0a; 8]);
                span.status = Status::error("request failed");
            }

            c.bench_with_input(
                BenchmarkId::new(format!("ETW/{layout}/Serialization"), case),
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
                BenchmarkId::new(format!("ETW/{layout}/{mode}/Export"), case),
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
criterion::criterion_group!(
    benches,
    windows::benchmark,
    windows::stages,
    windows::lifecycles
);
#[cfg(windows)]
criterion::criterion_main!(benches);

#[cfg(not(windows))]
fn main() {
    eprintln!("The ETW trace benchmarks require Windows; no measurements were taken.");
}
