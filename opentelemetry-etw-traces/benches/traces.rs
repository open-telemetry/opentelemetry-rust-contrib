/*
    Measured results: 2026-11-04. Original rows: rustc 1.99.0, before upstream merge.
    MixedMapped: rustc 1.99.1, measured separately after the merge.
    Microseconds per span/call; mean of two per-process Criterion slope estimates.

    | Layout | Case        | SDK-only | Serialize | Serialize+ETW | Write-only | End-to-end |
    |--------|-------------|----------|-----------|---------------|------------|------------|
    | Nested | 0 attrs     | 0.343    | 0.415     | 1.366         | 0.711      | 2.111      |
    | Nested | 3 mixed     | 0.412    | 0.467     | 1.496         | 0.818      | 2.191      |
    | Nested | 6 strings   | 0.446    | 0.512     | 1.554         | 0.806      | 2.333      |
    | Nested | ParentError | 0.411    | 0.504     | 1.497         | 0.735      | 2.378      |
    | Nested | MixedMapped | -        | 0.549     | 1.579         | -          | -          |
    | Geneva | 0 attrs     | 0.343    | 0.382     | 1.326         | 0.684      | 1.830      |
    | Geneva | 3 mixed     | 0.412    | 0.425     | 1.308         | 0.927      | 2.309      |
    | Geneva | 6 strings   | 0.446    | 0.480     | 1.492         | 0.675      | 2.209      |
    | Geneva | ParentError | 0.411    | 0.463     | 1.434         | 0.748      | 2.290      |
    | Geneva | MixedMapped | -        | 0.521     | 1.661         | -          | -          |

    SDK-only: recording/context/cleanup, no processor; shared across layouts.
    Serialize+ETW: enabled check, TLS builder, serialization, and real ETW write.
    Write-only: complete EventBuilder::write() on a pre-serialized event.
    End-to-end: actual SDK + serialization + ETW write + cleanup, measured directly.
    ParentError: 3 mixed attributes, a nonzero parent ID, and an error description.
    MixedMapped: 4 mapped HTTP/DB attributes + 3 custom attributes; "-" = not measured.
    MixedMapped used 100 samples, 1 s warm-up / 2 s measurement in two processes:
      & $exe MixedMapped --bench --warm-up-time 1 --measurement-time 2 --noplot
    File capture: 64 KiB buffers, min 8192 / max 16384, keyword 1, level 5.
    All 9,825,050 captured events (including warm-up) matched the validated fixture.

    One unpinned producer; Windows 11 Microsoft VM, EPYC 7763, 8 guest cores /
    16 logical CPUs, 63.95 GiB RAM. Accepted ETW captures reported zero loss.
    Columns come from separate runs and are NOT additive; write timings are noisy.
    See BENCHMARKS.md for commands, measurement boundaries, variability, and history.
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

    // Call only from selected benchmark callbacks, before timed work.
    fn wait_for_listener(setup: &Once, expected: bool, is_enabled: impl Fn() -> bool) {
        setup.call_once(|| {
            if expected {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !is_enabled() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        });
    }

    fn assert_listener_state(probe: &Provider, expected: bool) {
        assert_eq!(
            probe.enabled(Level::Verbose, 1),
            expected,
            "ETW listener state does not match OTEL_ETW_BENCH_MODE; \
             see BENCHMARKS.md for capture instructions"
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
            wait_for_listener(&listener_setup, enabled, || {
                probe.enabled(Level::Verbose, 1)
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
                        wait_for_listener(&listener_setup, enabled, || {
                            probe.enabled(Level::Verbose, 1)
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

        let setup_listener = || {
            wait_for_listener(&listener_setup, enabled, || exporter.enabled());
            assert_eq!(exporter.enabled(), enabled, "ETW listener state mismatch");
        };
        let cases = CASES
            .into_iter()
            .map(|(case, count)| (case, attributes(count)))
            .chain([(
                "MixedMapped",
                vec![
                    KeyValue::new("http.request.method", "GET"),
                    KeyValue::new("url.full", "https://example.test/items/42"),
                    KeyValue::new("http.response.status_code", 200_i64),
                    KeyValue::new("db.system.name", "postgresql"),
                    KeyValue::new("string_key", "benchmark-value"),
                    KeyValue::new("integer_key", 42_i64),
                    KeyValue::new("bool_key", true),
                ],
            )]);
        for (case, attributes) in cases {
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
            // The mixed fixture covers only serialization and enabled export.
            if case == "MixedMapped" && !enabled {
                continue;
            }
            c.bench_with_input(
                BenchmarkId::new(format!("ETW/{layout}/{mode}/Export"), case),
                &span,
                |b, span| {
                    setup_listener();
                    b.iter(|| exporter.export(black_box(span)));
                    assert_eq!(exporter.enabled(), enabled, "ETW listener state mismatch");
                },
            );
            if case == "MixedMapped" {
                continue;
            }
            c.bench_with_input(
                BenchmarkId::new(format!("ETW/{layout}/{mode}/WriteOnly"), case),
                &span,
                |b, span| {
                    setup_listener();
                    let mut event = EventBuilder::new();
                    exporter.serialize(&mut event, span);
                    assert_eq!(exporter.write(&mut event), 0, "ETW preflight write failed");
                    b.iter(|| exporter.write(black_box(&mut event)));
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
