//! Propagate the current OpenTelemetry context across the await points of an async function.
//!
//! The [`propagate_context`] attribute macro rewrites every `.await` in the annotated body so the
//! awaited future runs with the context the body started with. It does not create a span and it
//! does not change which span is current, so a caller stays in control of its own trace.
//!
//! # The problem it solves
//!
//! [`Context`][context] lives in a thread-local, and an async runtime does not carry that
//! thread-local across a suspension point. The context is therefore current for the first poll of
//! a future and gone for every later one. Work after the first `.await` then reads an empty
//! context: an outgoing request carries no `traceparent` header, and a child span attaches to
//! nothing.
//!
//! The usual fix is [`FutureExt::with_context`][future-ext] at every call site, which is easy to
//! forget and noisy to read. This macro applies it for you.
//!
//! # Usage
//!
//! ```
//! use opentelemetry_context_macro::propagate_context;
//!
//! # async fn charge(order: u64) -> u64 { order }
//! # async fn ship(order: u64) -> u64 { order }
//! #[propagate_context]
//! async fn place_order(order: u64) -> u64 {
//!     // Both calls see the context that `place_order` was called with, even though the runtime
//!     // drops it at the first suspension point.
//!     charge(order).await;
//!     ship(order).await
//! }
//! ```
//!
//! The annotated crate needs `opentelemetry` as a dependency, because the expansion names it.
//! The `futures` feature carries [`FutureExt`][future-ext] and is on by default.
//!
//! # How it works
//!
//! The macro makes two changes to the body:
//!
//! 1. It binds `__otel_cx` to [`Context::current()`][current] as the first statement of the async
//!    body. The body of an `async fn` runs on first poll, so this reads the context of the caller
//!    that polls the future, not of the code that created it.
//! 2. It rewrites every `.await` in that body from `future.await` to
//!    `FutureExt::with_context(future, __otel_cx.clone()).await`, which attaches the captured
//!    context for each poll of that future and detaches it again when the poll returns.
//!
//! Capturing once and reusing the clone is the point. Reading the current context at each await
//! instead would read an empty context after the first suspension, which is the problem this macro
//! exists to fix.
//!
//! The expansion writes every path in full, as `<_ as ::opentelemetry::context::FutureExt>::
//! with_context(..)` rather than a method call on an imported trait, and it imports nothing into
//! the annotated function. Instrumented code is free to import a different `with_context`, such as
//! the one on `anyhow::Context`, without the two becoming ambiguous. Application code has no such
//! constraint, so the examples here import [`FutureExt`][future-ext-trait] and call the method
//! directly.
//!
//! # `#[async_trait]` methods
//!
//! The macro also accepts the shape [`async_trait`] generates, which is a synchronous function
//! that returns `Box::pin(async move { .. })`. The capture goes inside that async block, not at
//! the top of the synchronous function.
//!
//! The distinction matters because the arguments of a call are evaluated before the call. In
//! `service.method().await` the synchronous `method()` runs before the surrounding
//! `with_context(..)` attaches anything, so a capture at the top of it would read the context of
//! whoever polls the caller, which is empty after the caller's first suspension. Inside the async
//! block the capture instead runs on first poll, under the attached context.
//!
//! # What it does not cover
//!
//! - **A span is never created.** Nothing is added to the trace, and the current span inside the
//!   body stays the caller's span. Create and activate a span yourself where you want one.
//! - **The body's own statements are not covered**, only the futures it awaits. A log line
//!   between two awaits still reads whatever context the runtime left attached. Attach the context
//!   for a whole body with [`FutureExt::with_context`][future-ext] on the outermost future when you
//!   need that.
//! - **The captured context wins at the await points.** A body that attaches a context of its own
//!   and then awaits sees that context in its own statements, while the awaited future sees the
//!   captured one. Call [`FutureExt::with_context`][future-ext] at the call site when a single call
//!   has to run under a different context.
//! - **Nested `async` blocks, closures and macro bodies are left alone.** An `async` block that is
//!   awaited inline is covered anyway, because the enclosing `.await` attaches the context for the
//!   whole poll. One that is spawned is not, and neither are the arms of `select!` or `join!`,
//!   since a macro body is opaque tokens at expansion time.
//! - **Nested `fn`, `impl`, `mod` and other item definitions are left alone.** Their bodies run
//!   with their own callers' contexts, and the captured binding is not in scope there.
//! - **A spawned task starts with an empty context.** Pass the context in explicitly:
//!
//! ```
//! # use opentelemetry::{context::FutureExt, Context};
//! # async fn work() {}
//! # async fn spawning() {
//! let task_cx = Context::current();
//! tokio::spawn(async move { work().await }.with_context(task_cx));
//! # }
//! ```
//!
//! [context]: https://docs.rs/opentelemetry/latest/opentelemetry/struct.Context.html
//! [current]: https://docs.rs/opentelemetry/latest/opentelemetry/struct.Context.html#method.current
//! [future-ext]: https://docs.rs/opentelemetry/latest/opentelemetry/context/trait.FutureExt.html#method.with_context
//! [future-ext-trait]: https://docs.rs/opentelemetry/latest/opentelemetry/context/trait.FutureExt.html
//! [`async_trait`]: https://docs.rs/async-trait

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    fold::Fold, parse_macro_input, Block, Expr, ExprAsync, ExprAwait, ExprClosure, Item, ItemFn,
    Stmt,
};

/// Propagate the current OpenTelemetry context across every await point of an async function.
///
/// The macro takes no arguments. It accepts an `async fn`, and the synchronous
/// `Box::pin(async move { .. })`-returning function that `#[async_trait]` generates from one.
///
/// ```
/// use opentelemetry_context_macro::propagate_context;
///
/// # async fn fetch_user(id: u64) -> String { id.to_string() }
/// # async fn fetch_orders(user: &str) -> usize { user.len() }
/// #[propagate_context]
/// async fn load_profile(id: u64) -> usize {
///     let user = fetch_user(id).await;
///     fetch_orders(&user).await
/// }
/// ```
///
/// See the [crate documentation](crate) for what the expansion looks like, and for the cases it
/// deliberately leaves alone.
#[proc_macro_attribute]
pub fn propagate_context(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        let message = "propagate_context takes no arguments";
        return TokenStream::from(quote! { ::core::compile_error!(#message); });
    }

    let input = parse_macro_input!(item as ItemFn);

    match transform_function(input) {
        Ok(output) => TokenStream::from(quote! { #output }),
        Err(error) => TokenStream::from(error.to_compile_error()),
    }
}

fn transform_function(mut func: ItemFn) -> syn::Result<ItemFn> {
    // The body of an `async fn` *is* the future, so a capture placed at its top runs on first
    // poll, under the context of whoever polls it.
    if func.sig.asyncness.is_some() {
        let body = AwaitTransformer.fold_block(*func.block);
        func.block = Box::new(capturing_block(&body));
        return Ok(func);
    }

    if !returns_boxed_future(&func.sig.output) {
        return Err(syn::Error::new_spanned(
            &func.sig,
            "propagate_context expects an `async fn`, or the `Pin<Box<dyn Future>>`-returning \
             function that #[async_trait] generates from one",
        ));
    }

    // A synchronous function returning `Pin<Box<dyn Future<..>>>`, which is the shape
    // `#[async_trait]` generates: a single `Box::pin(async move { .. })` statement. Rebuild the
    // outer block around the rewritten inner one, so the capture lands inside the future.
    let Some(inner_block) = async_trait_inner_block(&func.block) else {
        return Err(syn::Error::new_spanned(
            &func.sig,
            "propagate_context could not find a `Box::pin(async move { .. })` body in this \
             Pin<Box<dyn Future>>-returning function (the shape #[async_trait] generates)",
        ));
    };
    let inner_body = AwaitTransformer.fold_block(inner_block.clone());
    let new_inner = capturing_block(&inner_body);
    func.block = syn::parse_quote! {
        {
            Box::pin(async move #new_inner)
        }
    };
    Ok(func)
}

/// Whether `output` is (roughly) `Pin<Box<dyn Future<..>>>`, the return type `#[async_trait]`
/// generates for a desugared `async fn`.
///
/// This matches the outer `Pin` alone, whatever the path qualification, because `#[async_trait]`
/// always writes its generated types in full, as `::core::pin::Pin`, to stay independent of the
/// call site's imports. An ordinary synchronous function rarely returns a bare `Pin`, so the outer
/// type is specific enough on its own.
fn returns_boxed_future(output: &syn::ReturnType) -> bool {
    let syn::ReturnType::Type(_, ty) = output else {
        return false;
    };
    let syn::Type::Path(type_path) = ty.as_ref() else {
        return false;
    };
    type_path
        .path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "Pin")
}

/// Extracts the inner `async move { .. }` block from a body shaped like
/// `Box::pin(async move { .. })`, the sole statement `#[async_trait]` generates for a desugared
/// `async fn`.
fn async_trait_inner_block(block: &Block) -> Option<&Block> {
    let [Stmt::Expr(Expr::Call(call), _)] = block.stmts.as_slice() else {
        return None;
    };
    let mut args = call.args.iter();
    match (args.next(), args.next()) {
        (Some(Expr::Async(async_expr)), None) => Some(&async_expr.block),
        _ => None,
    }
}

/// Wraps `block` in one that captures the current context as `__otel_cx` first.
///
/// The name starts with an underscore so a body without a single await point, where the binding
/// goes unused, still compiles without a warning.
fn capturing_block(block: &Block) -> Block {
    syn::parse_quote! {
        {
            let __otel_cx = ::opentelemetry::Context::current();
            #block
        }
    }
}

/// Rewrites each `.await` to attach the captured context for the poll of the awaited future.
struct AwaitTransformer;

impl Fold for AwaitTransformer {
    fn fold_expr_await(&mut self, node: ExprAwait) -> ExprAwait {
        // Descend first, so an await nested in the awaited expression, as in
        // `outer(inner().await).await`, is rewritten too.
        let node = syn::fold::fold_expr_await(self, node);
        let base = node.base;

        ExprAwait {
            attrs: node.attrs,
            base: Box::new(syn::parse_quote! {
                <_ as ::opentelemetry::context::FutureExt>::with_context(
                    #base,
                    ::core::clone::Clone::clone(&__otel_cx),
                )
            }),
            dot_token: node.dot_token,
            await_token: node.await_token,
        }
    }

    /// Leaves a nested item definition, such as a `fn` or an `impl` block, untouched. Its body has
    /// its own callers and its own context, and the captured binding is not in scope inside it.
    fn fold_item(&mut self, item: Item) -> Item {
        item
    }

    /// Leaves a nested `async` block untouched. One that is awaited inline is already covered by
    /// the enclosing `.await`, and rewriting an `async move` block would move the captured
    /// context into it, taking it away from the rest of the body.
    fn fold_expr_async(&mut self, expr: ExprAsync) -> ExprAsync {
        expr
    }

    /// Leaves a closure untouched, for the same reason as an `async` block: a `move` closure would
    /// take the captured context with it.
    fn fold_expr_closure(&mut self, expr: ExprClosure) -> ExprClosure {
        expr
    }
}
