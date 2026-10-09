//! What `#[propagate_context]` costs at runtime, against no instrumentation and against the same
//! propagation written by hand.
//!
//! **Purpose**: track the overhead over time and catch regressions. The absolute numbers matter
//! less than the relative distance between the scenarios, and than the macro staying level with the
//! hand-written equivalent it replaces.
//!
//! ## Scenarios
//!
//! - **baseline**: the function with no propagation at all, as the control.
//! - **macro**: the same function annotated with `#[propagate_context]`.
//! - **manual-per-await**: the expansion written by hand, `FutureExt::with_context` on each awaited
//!   future. This is what the macro replaces, so the two should measure the same. A gap here is a
//!   bug in the macro rather than a cost of using it.
//! - **manual-whole-body**: the alternative under discussion in
//!   [#791](https://github.com/open-telemetry/opentelemetry-rust-contrib/issues/791): one
//!   `with_context` around the whole body instead of one per await. It attaches once per poll of the
//!   body rather than once per poll of each awaited future, and it also covers the body's own
//!   statements, so it is the interesting comparison for that decision.
//!
//! Each scenario runs twice, because the cost of carrying a context depends on what is in it:
//!
//! - **empty**: nothing is attached, which is what an uninstrumented process does. This isolates
//!   the machinery, since the context has no span to clone.
//! - **in-a-trace**: a context holding a span context, which is the case the macro exists for.
//!
//! ## What is being measured
//!
//! `Context::current()` reads a thread-local and clones, and each `with_context` attaches and
//! detaches around a poll. The body awaits four futures that are ready on first poll and one that
//! suspends once, so the measurement covers both a poll that returns immediately and a poll after a
//! suspension. No SDK, exporter or tracer provider is involved: nothing here creates a span, so a
//! provider would only add noise from a component the macro does not touch.
//!
//! ## Run
//!
//! ```sh
//! cargo bench --bench propagation -p opentelemetry-context-macro
//! ```
//!
//! ## Reference numbers
//!
//! Latest measurements (criterion median), for a body with five await points:
//!
//! | Scenario          | empty  | vs baseline | in a trace | vs baseline |
//! | ----------------- | ------ | ----------- | ---------- | ----------- |
//! | baseline          |  61 ns | —           |      66 ns | —           |
//! | macro             | 126 ns | +65 ns      |     150 ns | +84 ns      |
//! | manual-per-await  | 126 ns | +65 ns      |     151 ns | +85 ns      |
//! | manual-whole-body |  78 ns | +17 ns      |      83 ns | +17 ns      |
//!
//! Two things to read from this. The macro measures the same as the hand-written per-await form, so
//! it costs nothing beyond the propagation it writes for you. And the per-await form costs about
//! four times the whole-body form, because it attaches once per awaited future rather than once per
//! poll of the body, which is a point for the whole-body shape in #791 on top of the coverage
//! argument.
//!
//! Captured on: MacBook Pro, Apple M4 Max (12P + 4E cores), 48 GB RAM, macOS 26.6.2,
//! rustc 1.98.0, OpenTelemetry 0.32.

use std::{
    future::Future,
    hint::black_box,
    pin::Pin,
    task::{Context as TaskContext, Poll},
    time::Instant,
};

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use opentelemetry::{
    context::FutureExt,
    trace::{SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState},
    Context,
};
use opentelemetry_context_macro::propagate_context;

/// Suspends once, so the body is polled again after the runtime has taken the context away.
#[derive(Default)]
struct SuspendOnce {
    suspended: bool,
}

impl Future for SuspendOnce {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, task_cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        if self.suspended {
            return Poll::Ready(());
        }
        self.suspended = true;
        task_cx.waker().wake_by_ref();
        Poll::Pending
    }
}

/// The unit of work each scenario awaits. Ready on first poll, so the measurement is dominated by
/// the propagation rather than by the work.
async fn step(value: u64) -> u64 {
    value + 1
}

async fn baseline(value: u64) -> u64 {
    let value = step(value).await;
    let value = step(value).await;
    SuspendOnce::default().await;
    let value = step(value).await;
    step(value).await
}

#[propagate_context]
async fn with_macro(value: u64) -> u64 {
    let value = step(value).await;
    let value = step(value).await;
    SuspendOnce::default().await;
    let value = step(value).await;
    step(value).await
}

/// The macro's expansion, written out. Kept in step with `capturing_block` and `AwaitTransformer`
/// in the crate: if the macro changes shape, this changes with it, or the comparison stops meaning
/// anything.
async fn manual_per_await(value: u64) -> u64 {
    let cx = Context::current();
    let value = step(value).with_context(cx.clone()).await;
    let value = step(value).with_context(cx.clone()).await;
    SuspendOnce::default().with_context(cx.clone()).await;
    let value = step(value).with_context(cx.clone()).await;
    step(value).with_context(cx).await
}

/// One attach around the whole body, the alternative shape under discussion.
async fn manual_whole_body(value: u64) -> u64 {
    let cx = Context::current();
    async move {
        let value = step(value).await;
        let value = step(value).await;
        SuspendOnce::default().await;
        let value = step(value).await;
        step(value).await
    }
    .with_context(cx)
    .await
}

/// A context holding a span context, standing in for a caller that is already in a trace.
///
/// A fixed span context needs no tracer provider, so nothing in the measurement depends on SDK
/// sampling or export behaviour.
fn context_in_a_trace() -> Context {
    Context::new().with_remote_span_context(SpanContext::new(
        TraceId::from_bytes([1; 16]),
        SpanId::from_bytes([2; 8]),
        TraceFlags::SAMPLED,
        true,
        TraceState::NONE,
    ))
}

/// The scenarios, as futures behind one pointer type so a single loop can time them all.
type Scenario = fn(u64) -> Pin<Box<dyn Future<Output = u64>>>;

fn scenarios() -> [(&'static str, Scenario); 4] {
    [
        ("baseline", |value| Box::pin(baseline(value))),
        ("macro", |value| Box::pin(with_macro(value))),
        ("manual-per-await", |value| {
            Box::pin(manual_per_await(value))
        }),
        ("manual-whole-body", |value| {
            Box::pin(manual_whole_body(value))
        }),
    ]
}

fn benchmark_propagation(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("the runtime builds");

    let mut group = c.benchmark_group("context-propagation");
    group.throughput(Throughput::Elements(1));

    for (context_name, context) in [
        ("empty", Context::new()),
        ("in-a-trace", context_in_a_trace()),
    ] {
        for (scenario_name, scenario) in scenarios() {
            let id = BenchmarkId::new(scenario_name, context_name);
            group.bench_function(id, |bencher| {
                bencher.to_async(&runtime).iter_custom(|iterations| {
                    let context = context.clone();
                    async move {
                        // The caller's context is attached outside the timed section, the way a
                        // server's middleware attaches it before a handler runs.
                        let _guard = context.attach();

                        let start = Instant::now();
                        for iteration in 0..iterations {
                            black_box(scenario(black_box(iteration)).await);
                        }
                        start.elapsed()
                    }
                });
            });
        }
    }

    group.finish();
}

criterion_group!(benches, benchmark_propagation);
criterion_main!(benches);
