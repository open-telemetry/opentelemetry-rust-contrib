//! Shows what `#[propagate_context]` does for work that happens after an await point.
//!
//! Run with `cargo run --example basic -p opentelemetry-context-macro`.
//!
//! The example plays the two halves of a service. A request arrives with a `traceparent` header,
//! so the work belongs to a trace that started elsewhere. The handler waits on something slow and
//! then calls another service, which has to send that same trace on in a header of its own.
//!
//! The wait is what makes this interesting. The OpenTelemetry context lives in a thread-local, and
//! nothing restores that thread-local when a suspended future is polled again. The handler
//! therefore resumes with an empty context unless something puts the context back. The macro does
//! that: it captures the context on the handler's first poll and attaches it again for every future
//! the handler awaits, which is why the outgoing header repeats the inbound trace id.

use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context as TaskContext, Poll, Wake, Waker},
};

use opentelemetry::{
    propagation::{Extractor, Injector, TextMapPropagator},
    Context,
};
use opentelemetry_context_macro::propagate_context;
use opentelemetry_sdk::propagation::TraceContextPropagator;

/// The headers of the request that arrives, already part of a trace.
fn inbound_headers() -> HashMap<String, String> {
    let mut headers = HashMap::new();
    headers.insert(
        "traceparent".to_owned(),
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".to_owned(),
    );
    headers
}

/// A `HashMap` the propagator can read from and write to.
struct HeaderMap(HashMap<String, String>);

impl Extractor for HeaderMap {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(String::as_str).collect()
    }
}

impl Injector for HeaderMap {
    fn set(&mut self, key: &str, value: String) {
        self.0.insert(key.to_owned(), value);
    }
}

/// Stands in for an HTTP client: reads the current context and writes it into request headers.
///
/// A real client does the same thing, which is why the context has to be right at the moment the
/// call is made, and not only at the moment the handler started.
async fn call_other_service() -> String {
    let mut headers = HeaderMap(HashMap::new());
    TraceContextPropagator::new().inject_context(&Context::current(), &mut headers);

    headers
        .0
        .get("traceparent")
        .cloned()
        .unwrap_or_else(|| "<none>".to_owned())
}

/// The handler that carries the context past the wait.
///
/// The macro creates no span. Whether a function deserves a span of its own stays a decision for
/// the code, so create and activate one where you want one.
#[propagate_context]
async fn handle_request() -> String {
    slow_work().await;
    call_other_service().await
}

/// Suspends once, the way a database query or a queue would.
#[derive(Default)]
struct SlowWork {
    suspended: bool,
}

impl Future for SlowWork {
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

fn slow_work() -> SlowWork {
    SlowWork::default()
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

/// Runs `future` the way work runs when nothing re-attaches the context: attached while the work
/// starts, gone from every poll after that.
///
/// Middleware such as `opentelemetry-instrumentation-tower` attaches the context for each poll of
/// the future it wraps, which covers the handlers it polls directly. Work it cannot reach, such as
/// a future a combinator holds or a task that took the context in by hand, is polled like this.
fn run_losing_the_context<F: Future>(future: F, cx: Context) -> F::Output {
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

fn main() {
    let headers = HeaderMap(inbound_headers());
    let inbound_cx = TraceContextPropagator::new().extract(&headers);

    let outbound = run_losing_the_context(handle_request(), inbound_cx);

    println!("inbound  traceparent: {:?}", headers.get("traceparent"));
    println!("outbound traceparent: {outbound}");
}
