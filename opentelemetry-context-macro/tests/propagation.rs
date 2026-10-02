//! What `#[propagate_context]` guarantees about context propagation.
//!
//! Every test drives its future by hand instead of on an async runtime, because the guarantee is
//! about what happens when the context is no longer attached. [`drive_after_losing_context`]
//! reproduces exactly that: the first poll runs with a context attached, every later poll runs
//! with none, which is what a runtime does to any future that suspends.

use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context as TaskContext, Poll, Wake, Waker},
};

use async_trait::async_trait;
use opentelemetry::{
    trace::{SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState},
    Context,
};
use opentelemetry_context_macro::propagate_context;

/// The span id of the context the caller starts with, as its `traceparent` form.
const CALLER_SPAN_ID: &str = "00f067aa0ba902b7";

/// The span id of an empty context, which is what a body reads once the context is lost.
const NO_SPAN_ID: &str = "0000000000000000";

/// A context naming a fixed remote span, standing in for a caller that is already in a trace.
///
/// A fixed span context keeps the expected values in each test literal, and needs no SDK, tracer
/// provider or exporter to produce one.
fn caller_context() -> Context {
    Context::new().with_remote_span_context(SpanContext::new(
        TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e4736").expect("valid trace id"),
        SpanId::from_hex(CALLER_SPAN_ID).expect("valid span id"),
        TraceFlags::SAMPLED,
        true,
        TraceState::NONE,
    ))
}

/// The span id that `Context::current()` names right now, as its `traceparent` form.
fn current_span_id() -> String {
    Context::current()
        .span()
        .span_context()
        .span_id()
        .to_string()
}

/// Reports the span id current at the time this future is polled, rather than at the time it is
/// created, which is what makes it an observer of the attached context.
async fn observe() -> String {
    current_span_id()
}

/// Hands `value` back after a poll, so an observation can be made behind another await point.
async fn relay(value: String) -> String {
    value
}

/// A future that suspends exactly once, so the poll after it stands for the poll a runtime makes
/// after a suspension: without any context attached.
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

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

/// Runs `future` to completion with `cx` attached for its first poll only.
///
/// An async runtime attaches nothing of its own, so a future that suspends is polled again with an
/// empty context. Any context the body still sees after that point is one it captured itself,
/// which is what these tests measure.
fn drive_after_losing_context<F: Future>(future: F, cx: Context) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut task_cx = TaskContext::from_waker(&waker);
    let mut future = Box::pin(future);

    let first_poll = {
        let _guard = cx.attach();
        future.as_mut().poll(&mut task_cx)
    };
    if let Poll::Ready(output) = first_poll {
        return output;
    }

    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut task_cx) {
            return output;
        }
    }
}

/// The context a body starts with must reach every future it awaits, however the body is shaped.
///
/// Each case awaits an observer and reports the span id that observer saw. The unannotated case is
/// the control: it shows that the harness really does take the context away, so a passing
/// annotated case means the macro carried it and not that the context was never lost.
#[test]
fn awaited_futures_see_the_context_the_body_started_with() {
    struct TestCase {
        name: &'static str,
        observe: fn() -> Pin<Box<dyn Future<Output = String>>>,
        expected_span_id: &'static str,
    }

    async fn unannotated_after_suspension() -> String {
        SuspendOnce::default().await;
        observe().await
    }

    #[propagate_context]
    async fn before_suspension() -> String {
        observe().await
    }

    #[propagate_context]
    async fn after_suspension() -> String {
        SuspendOnce::default().await;
        observe().await
    }

    #[propagate_context]
    async fn without_await_points() -> String {
        current_span_id()
    }

    #[propagate_context]
    async fn await_nested_in_awaited_expression() -> String {
        SuspendOnce::default().await;
        relay(observe().await).await
    }

    #[propagate_context]
    async fn await_in_loop_and_match() -> String {
        SuspendOnce::default().await;

        let mut observed = String::new();
        for _ in 0..2 {
            observed = observe().await;
        }

        match observed.is_empty() {
            true => observe().await,
            false => observed,
        }
    }

    #[propagate_context]
    async fn await_through_nested_item_and_async_block() -> String {
        // A nested item is left as written, so its body must not reference the captured binding.
        // An inline `async` block is left as written too, and is covered anyway by the `.await`
        // that polls it.
        async fn nested_item() -> String {
            observe().await
        }

        SuspendOnce::default().await;
        async { nested_item().await }.await
    }

    #[propagate_context]
    async fn await_after_two_suspensions() -> String {
        SuspendOnce::default().await;
        let first = observe().await;
        SuspendOnce::default().await;
        let second = observe().await;

        assert_eq!(first, second, "the captured context must survive reuse");
        second
    }

    #[async_trait]
    trait Probe {
        async fn observe_through_trait(&self) -> String;
    }

    struct TraitProbe;

    #[async_trait]
    impl Probe for TraitProbe {
        #[propagate_context]
        async fn observe_through_trait(&self) -> String {
            SuspendOnce::default().await;
            observe().await
        }
    }

    #[propagate_context]
    async fn call_trait_method_after_suspension() -> String {
        SuspendOnce::default().await;
        TraitProbe.observe_through_trait().await
    }

    let test_cases = vec![
        TestCase {
            name: "unannotated body, awaiting after a suspension",
            observe: || Box::pin(unannotated_after_suspension()),
            expected_span_id: NO_SPAN_ID,
        },
        TestCase {
            name: "annotated body, awaiting before any suspension",
            observe: || Box::pin(before_suspension()),
            expected_span_id: CALLER_SPAN_ID,
        },
        TestCase {
            name: "annotated body, awaiting after a suspension",
            observe: || Box::pin(after_suspension()),
            expected_span_id: CALLER_SPAN_ID,
        },
        TestCase {
            name: "annotated body without a single await point",
            observe: || Box::pin(without_await_points()),
            expected_span_id: CALLER_SPAN_ID,
        },
        TestCase {
            name: "await nested in the expression another await consumes",
            observe: || Box::pin(await_nested_in_awaited_expression()),
            expected_span_id: CALLER_SPAN_ID,
        },
        TestCase {
            name: "await inside a loop and a match arm",
            observe: || Box::pin(await_in_loop_and_match()),
            expected_span_id: CALLER_SPAN_ID,
        },
        TestCase {
            name: "await reaching through a nested item and an inline async block",
            observe: || Box::pin(await_through_nested_item_and_async_block()),
            expected_span_id: CALLER_SPAN_ID,
        },
        TestCase {
            name: "await after the body suspended twice",
            observe: || Box::pin(await_after_two_suspensions()),
            expected_span_id: CALLER_SPAN_ID,
        },
        TestCase {
            name: "async_trait method called after the caller suspended",
            observe: || Box::pin(call_trait_method_after_suspension()),
            expected_span_id: CALLER_SPAN_ID,
        },
    ];

    for TestCase {
        name,
        observe,
        expected_span_id,
    } in test_cases
    {
        let observed = drive_after_losing_context(observe(), caller_context());

        assert_eq!(observed, expected_span_id, "Failed case: {name}");
    }
}

/// The macro must not touch the trace itself: no span is created, and the current span stays the
/// caller's own.
///
/// A created span would show up as a different span id, and a replaced one as a different trace id
/// or sampling decision, so comparing the whole span context covers both.
#[test]
fn the_current_span_stays_the_callers_span() {
    #[propagate_context]
    async fn observe_span_context() -> (String, String, bool) {
        SuspendOnce::default().await;
        observe_span_context_inner().await
    }

    async fn observe_span_context_inner() -> (String, String, bool) {
        let span_context = Context::current().span().span_context().clone();
        (
            span_context.trace_id().to_string(),
            span_context.span_id().to_string(),
            span_context.is_sampled(),
        )
    }

    let observed = drive_after_losing_context(observe_span_context(), caller_context());

    assert_eq!(
        observed,
        (
            "4bf92f3577b34da6a3ce929d0e0e4736".to_owned(),
            CALLER_SPAN_ID.to_owned(),
            true
        )
    );
}

/// Rewriting the await points must leave the function's own contract alone: the value it returns,
/// and the early exit `?` performs on an error.
#[test]
fn the_function_keeps_its_return_value_and_early_exits() {
    #[propagate_context]
    async fn succeeds() -> Result<String, String> {
        SuspendOnce::default().await;
        let observed = fallible(Ok(observe().await)).await?;
        Ok(observed)
    }

    #[propagate_context]
    async fn fails() -> Result<String, String> {
        SuspendOnce::default().await;
        let observed = fallible(Err("rejected".to_owned())).await?;

        unreachable!("`?` must return before this point, observed {observed}");
    }

    async fn fallible(outcome: Result<String, String>) -> Result<String, String> {
        outcome
    }

    let succeeded = drive_after_losing_context(succeeds(), caller_context());
    let failed = drive_after_losing_context(fails(), caller_context());

    assert_eq!(
        (succeeded, failed),
        (Ok(CALLER_SPAN_ID.to_owned()), Err("rejected".to_owned()))
    );
}

/// The captured context wins at the await points, even over a context the body attached itself.
///
/// This is a consequence of capturing once, and worth pinning because it is surprising: a body that
/// attaches a child context and then awaits sees that child in its own statements, while the future
/// it awaits sees the captured one. Reach for `FutureExt::with_context` at the call site when a
/// single call has to run under a different context.
#[test]
fn a_context_the_body_attaches_does_not_reach_the_futures_it_awaits() {
    const LOCAL_SPAN_ID: &str = "00f067aa0ba902b8";

    #[propagate_context]
    async fn attaches_a_context_of_its_own() -> (String, String) {
        SuspendOnce::default().await;

        let local_cx = Context::new().with_remote_span_context(SpanContext::new(
            TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e4736").expect("valid trace id"),
            SpanId::from_hex(LOCAL_SPAN_ID).expect("valid span id"),
            TraceFlags::SAMPLED,
            true,
            TraceState::NONE,
        ));
        let _guard = local_cx.attach();

        (current_span_id(), observe().await)
    }

    let observed = drive_after_losing_context(attaches_a_context_of_its_own(), caller_context());

    assert_eq!(
        observed,
        (LOCAL_SPAN_ID.to_owned(), CALLER_SPAN_ID.to_owned())
    );
}
