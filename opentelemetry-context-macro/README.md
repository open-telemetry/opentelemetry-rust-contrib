# OpenTelemetry Context Propagation Macro

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

| Status        |                                              |
| ------------- |----------------------------------------------|
| Stability     | alpha                                        |
| Owners        | [Jan Steinke](https://github.com/jan-xyz)    |

An attribute macro that carries the OpenTelemetry context across the await points of an async
function, so work that happens after a suspension stays in the trace it started in.

[![Crates.io: opentelemetry-context-macro](https://img.shields.io/crates/v/opentelemetry-context-macro.svg)](https://crates.io/crates/opentelemetry-context-macro)
[![Documentation](https://docs.rs/opentelemetry-context-macro/badge.svg)](https://docs.rs/opentelemetry-context-macro)
[![Slack](https://img.shields.io/badge/slack-@cncf/otel/rust-brightgreen.svg?logo=slack)](https://cloud-native.slack.com/archives/C03GDP0H023)

## The problem

`opentelemetry::Context` lives in a thread-local, and nothing restores that thread-local when an
async runtime polls a suspended future again. The context is therefore current for the first poll of
a future and gone for every later one. Work after the first `.await` then reads an empty context: an
outgoing request carries no `traceparent` header, and a child span attaches to nothing.

The fix is `FutureExt::with_context` at every call site, which is easy to forget and noisy to read:

```rust,ignore
use opentelemetry::{context::FutureExt, Context};

async fn place_order(order: u64) -> u64 {
    let cx = Context::current();

    charge(order).with_context(cx.clone()).await;
    ship(order).with_context(cx).await
}
```

## Usage

Add the dependency:

```toml
[dependencies]
opentelemetry = "0.32"
opentelemetry-context-macro = "0.1"
```

Then annotate the function:

```rust,ignore
use opentelemetry_context_macro::propagate_context;

#[propagate_context]
async fn place_order(order: u64) -> u64 {
    charge(order).await;
    ship(order).await
}
```

The annotated crate needs `opentelemetry` as a dependency, because the expansion names it. The
`futures` feature carries `FutureExt` and is on by default.

The macro also accepts the shape [`async_trait`](https://docs.rs/async-trait) generates, which is a
synchronous function returning `Box::pin(async move { .. })`:

```rust,ignore
#[async_trait]
impl Warehouse for Postgres {
    #[propagate_context]
    async fn reserve(&self, order: u64) -> u64 {
        self.query(order).await
    }
}
```

Run the worked example, which sends an inbound trace on in an outgoing header after a wait:

```sh
cargo run --example basic -p opentelemetry-context-macro
```

## How it works

The macro makes two changes to the body:

1. It binds `__otel_cx` to `Context::current()` as the first statement of the async body. The body
   of an `async fn` runs on first poll, so this reads the context of the caller that polls the
   future, not of the code that created it.
2. It rewrites every `.await` in that body from `future.await` to
   `FutureExt::with_context(future, __otel_cx.clone()).await`, which attaches the captured context
   for each poll of that future and detaches it again when the poll returns.

Capturing once and reusing the clone is the point. Reading the current context at each await instead
would read an empty context after the first suspension, which is the problem the macro exists to
fix.

The expansion writes every path in full, as `<_ as ::opentelemetry::context::FutureExt>::with_context(..)`
rather than a method call on an imported trait, and it imports nothing into the annotated function.
Instrumented code is free to import a different `with_context`, such as the one on `anyhow::Context`,
without the two becoming ambiguous. Application code has no such constraint, so the examples here
import `FutureExt` and call the method directly.

### `#[async_trait]` methods

`async_trait` desugars an `async fn` into a synchronous function that returns
`Box::pin(async move { .. })`. The capture goes inside that async block, not at the top of the
synchronous function, because the arguments of a call are evaluated before the call. In
`warehouse.reserve(order).await` the synchronous `reserve()` runs before the surrounding
`with_context(..)` attaches anything, so a capture at the top of it would read the context of
whoever polls the caller, which is empty after the caller's first suspension.

## What it does not do

- **It never creates a span.** Nothing is added to the trace, and the current span inside the body
  stays the caller's span. Create and activate a span yourself where you want one.
- **It does not cover the body's own statements**, only the futures the body awaits. A log line
  between two awaits still reads whatever context the runtime left attached. Attach the context for
  a whole body with `FutureExt::with_context` on the outermost future when you need that.
- **The captured context wins at the await points**, even over a context the body attached itself.
  Call `FutureExt::with_context` at the call site when a single call has to run under a different
  context.
- **Nested `async` blocks, closures and macro bodies are left alone.** An `async` block that is
  awaited inline is covered anyway, because the enclosing `.await` attaches the context for the whole
  poll. One that is spawned is not, and neither are the arms of `select!` or `join!`, since a macro
  body is opaque tokens at expansion time.
- **Nested `fn`, `impl`, `mod` and other item definitions are left alone.** Their bodies run with
  their own callers' contexts.
- **A spawned task starts with an empty context.** Pass the context in explicitly:

```rust,ignore
use opentelemetry::{context::FutureExt, Context};

let task_cx = Context::current();
tokio::spawn(async move { work().await }.with_context(task_cx));
```

## Related work

- [`tracing::instrument`](https://docs.rs/tracing/latest/tracing/attr.instrument.html) does the
  same job for `tracing` spans, and creates a span as well.
- [`opentelemetry-instrumentation-tower`](../opentelemetry-instrumentation-tower) attaches the
  context for each poll of the future it wraps, which covers the handlers it polls directly. This
  macro covers the calls that middleware cannot reach.
