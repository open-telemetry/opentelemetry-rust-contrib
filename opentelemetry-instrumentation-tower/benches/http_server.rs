//! Benchmarks for the tower OpenTelemetry HTTP **server** layer overhead.
//!
//! This benchmark measures the **pure instrumentation overhead** of the `http::server::Layer`
//! middleware. It uses tower's service utilities to call handlers directly, bypassing
//! the network stack (TCP/HTTP parsing). This isolates the middleware cost and provides
//! consistent, reproducible measurements.
//!
//! **Purpose**: Track instrumentation performance over time and identify regressions.
//! The absolute numbers are less important than relative changes between versions.
//!
//! ## Scenarios
//!
//! - **Baseline**: No middleware (control measurement)
//! - **No-op**: `http::server::Layer` present, but both tracer and meter are no-ops
//! - **Tracing**: `http::server::Layer` with active tracer, no-op meter (all spans sampled)
//! - **Tracing (query)**: Same, but the request target carries a query string
//!   that holds no sensitive parameter
//! - **Tracing (query, redacted)**: Same, but the query string holds a credential,
//!   so the layer has to redact it
//! - **Tracing (sampled-out)**: Same, but with `AlwaysOff` sampler (all spans dropped)
//! - **Metrics**: `http::server::Layer` with active meter, no-op tracer (no spans created)
//! - **Tracing + Metrics**: `http::server::Layer` with both active tracer and active meter
//! - **Tracing + Metrics (500)**: Same as above, but the handler answers with a
//!   500 response, to baseline the extra `error.type` attribute, span status,
//!   and metric label
//!
//! Each tracing scenario sets the global `TracerProvider` before building the layer so
//! that no tracer state leaks between scenarios.  Scenarios that do not need traces use
//! `NoopTracerProvider` (a true no-op) to reset the global.  No meter reset is needed
//! because the global meter is never set in tracing-only scenarios, leaving meter
//! instruments as no-ops by default.
//!
//! ## Known Performance Characteristics
//!
//! ### Middleware-internal (optimizable by this crate)
//!
//! - **Heap allocations per request**: `path`, `URI` and (for non-common
//!   methods or schemes) the `method`/`scheme` strings still `to_string()`
//!   / `to_owned()` into a new `String`. The `method` and `scheme` labels
//!   are promoted to `&'static str` for the common HTTP methods
//!   (GET/POST/PUT/DELETE/HEAD/OPTIONS/PATCH/CONNECT/TRACE) and the `http`
//!   and `https` schemes, so the corresponding `KeyValue::clone()` is
//!   allocation-free in the hot path.
//! - **`Vec<KeyValue>` allocations**: `span_attributes` is still built per
//!   request. `label_superset` (the metric label set) is pre-sized to its
//!   final capacity and ownership of the per-request `KeyValue`s is moved
//!   into it instead of being cloned, so it only allocates its backing
//!   buffer.
//! - **Unconditional attribute extraction**: all `KeyValue` attributes are built
//!   on every request without first checking whether the tracer or meter are
//!   no-ops.
//!
//! ### Tower / OTel SDK (outside this middleware's control)
//!
//! - **`Box::pin`** wrapping the response future — heap-allocates on every
//!   request. This is a consequence of the Tower `Service` trait design.
//!   (See [PR #561](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/561)
//!   for a `ResponseFuture`-based alternative.)
//! - **`global::get_text_map_propagator`** — acquires a `RwLock` on every
//!   request. Caching would require a new `with_propagator` builder method
//!   since there is no public API to obtain an owned/`Arc`'d propagator from
//!   the global.
//! - **`OtelContext::current()`** — thread-local access + clone in
//!   `finalize_request`.
//!
//! ## Run
//!
//! ```shell
//! cargo bench --bench http_server -p opentelemetry-instrumentation-tower
//! ```
//!
//! ### Minimize Benchmark Fluctuation
//! As these benchmarks cover a very small codepath,
//! measurements can fluctuate significantly between runs without any code changes.
//!
//! Minimize noise by increasing samples collected to 1000 (from the default 100):
//! ```shell
//! cargo bench --bench http_server -p opentelemetry-instrumentation-tower -- --sample-size 1000
//! ```
//!
//! Further minimize noise from interrupts by avoiding CPU 0,
//! or CPUs 0 and 1 on a machine with two logical cores per physical core:
//! ```shell
//! taskset -c 2,3 cargo bench --bench http_server -p opentelemetry-instrumentation-tower -- --sample-size 1000
//! ```
//!
//! Despite these mitigations, Criterion often measures 3-6% changes in medians
//! between consecutive runs without any code changes.
//!
//! ## Reference Numbers
//!
//! Latest measurements (criterion median):
//!
//! | Scenario                | Median   | vs baseline |
//! | ----------------------- | -------- | ----------- |
//! | baseline                |   48 ns  | —           |
//! | noop                    |  381 ns  | +333 ns     |
//! | tracing                 |  489 ns  | +442 ns     |
//! | tracing-query           |  561 ns  | +513 ns     |
//! | tracing-query-redacted  |  594 ns  | +546 ns     |
//! | tracing-sampled-out     |  399 ns  | +351 ns     |
//! | metrics                 |  687 ns  | +640 ns     |
//! | tracing + metrics       |  823 ns  | +775 ns     |
//! | tracing + metrics-500   |  858 ns  | +810 ns     |
//!
//! `tracing + metrics-500` isolates the `error.type` code path: the extra
//! span attribute, span status, and metric label on a 5xx response cost
//! about 35 ns more than `tracing + metrics`.
//!
//! Captured on: MacBook Pro, Apple M4 Max (12P + 4E cores), 48 GB RAM,
//! macOS 26.7, rustc 1.98.0, OpenTelemetry 0.32.
//!

use benchfmt::{Bencher, CountingAlloc, Suite};
use opentelemetry::global;
use opentelemetry::trace::noop::NoopTracerProvider;
use opentelemetry_instrumentation_tower::http::server::LayerBuilder;
use opentelemetry_sdk::{
    metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider},
    trace::{Sampler, SdkTracerProvider},
};
use std::convert::Infallible;
use std::fmt::Debug;
use std::future::Future;
use std::hint::black_box;
use std::time::Duration;
use tokio::runtime::Runtime;
use tower::{Service, ServiceBuilder, ServiceExt};

/// Counts allocations per thread. The benchmark loop drives every future with
/// `block_on` on the calling thread, so no allocation escapes to a worker thread.
#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc::new();

const TARGET: &str = "http://example.com/users/123";
const TARGET_QUERY: &str = "http://example.com/users/123?fields=name&verbose=true";
const TARGET_QUERY_REDACTED: &str = "http://example.com/users/123?fields=name&sig=secret-signature";

/// Minimal handler — returns an empty body to keep baseline noise as low as possible.
async fn handler(_req: http::Request<String>) -> Result<http::Response<String>, Infallible> {
    Ok(http::Response::new(String::new()))
}

/// Handler that always answers with a 500. Isolates the additional cost of
/// the `error.type` code path: the extra span attribute, the span status,
/// and the extra metric label.
async fn error_handler(_req: http::Request<String>) -> Result<http::Response<String>, Infallible> {
    Ok(http::Response::builder()
        .status(http::StatusCode::INTERNAL_SERVER_ERROR)
        .body(String::new())
        .unwrap())
}

fn build_request(target: &'static str) -> http::Request<String> {
    http::Request::builder()
        .method("GET")
        .uri(target)
        .body(String::new())
        .unwrap()
}

/// Setup tracer provider with no-op processor (measures instrumentation overhead only)
fn setup_tracer() -> SdkTracerProvider {
    // No exporter = no-op processing, just measures instrumentation overhead
    let provider = SdkTracerProvider::builder().build();
    global::set_tracer_provider(provider.clone());
    provider
}

/// Setup tracer provider with AlwaysOff sampler (all spans are dropped).
/// This measures the overhead of the non-sampled code path.
fn setup_sampled_out_tracer() -> SdkTracerProvider {
    let provider = SdkTracerProvider::builder()
        .with_sampler(Sampler::AlwaysOff)
        .build();
    global::set_tracer_provider(provider.clone());
    provider
}

/// Reset global tracer provider to a true no-op instance.
/// Call this in scenarios that should not generate traces.
fn noop_tracer() {
    global::set_tracer_provider(NoopTracerProvider::new());
}

/// Setup meter provider with in-memory exporter (minimal I/O overhead)
fn setup_meter() -> (SdkMeterProvider, InMemoryMetricExporter) {
    let exporter = InMemoryMetricExporter::default();
    // Use very long interval to ensure no timer-based exports during benchmark
    let reader = PeriodicReader::builder(exporter.clone())
        .with_interval(Duration::from_secs(3600))
        .build();
    let provider = SdkMeterProvider::builder().with_reader(reader).build();
    global::set_meter_provider(provider.clone());
    (provider, exporter)
}

/// Wrap `inner` in the server layer under test.
/// Call this after the global tracer and meter providers are set.
fn layered<F, Fut>(
    inner: F,
) -> impl Service<http::Request<String>, Response = http::Response<String>, Error = Infallible>
where
    F: FnMut(http::Request<String>) -> Fut,
    Fut: Future<Output = Result<http::Response<String>, Infallible>> + Send,
{
    let layer = LayerBuilder::builder().build().unwrap();
    ServiceBuilder::new()
        .layer(layer)
        .service(tower::service_fn(inner))
}

/// Measurement loop shared by all scenarios.
///
/// Everything before the loop is one-time setup and stays out of the measurement.
/// The whole loop runs inside a single `block_on`, so the runtime entry cost is paid once
/// and not once per request.
/// Each request is a clone of one prebuilt request.
/// The `baseline` scenario pays the same clone cost, so compare scenarios against it.
fn measure<S>(b: &mut Bencher<'_>, rt: &Runtime, mut service: S, target: &'static str)
where
    S: Service<http::Request<String>, Response = http::Response<String>>,
    S::Error: Debug,
{
    let request = build_request(target);
    rt.block_on(async {
        service.ready().await.unwrap();
        b.report_allocs();
        while b.keep_running() {
            let response = service.call(request.clone()).await;
            black_box(response.unwrap());
        }
    });
}

fn main() {
    // A current-thread runtime keeps all work, and so all allocations, on the calling thread.
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();

    Suite::from_env("opentelemetry-instrumentation-tower")
        .with_alloc(&ALLOC)
        // Scenario 1: Baseline - no middleware
        .bench("HttpServer/scenario=baseline", |b| {
            measure(b, &rt, tower::service_fn(handler), TARGET);
        })
        // Scenario 2: Middleware present, but both tracer and meter are no-ops.
        // Measures the pure overhead of the server::Layer machinery (attribute extraction,
        // context propagation hooks, etc.) when no real telemetry is produced.
        .bench("HttpServer/scenario=noop", |b| {
            // reset any tracer left from a previous run
            // meter is not set, so meter instruments are already no-op
            noop_tracer();
            measure(b, &rt, layered(handler), TARGET);
        })
        // Scenario 3: Tracing only (global meter not set, so meter instruments are no-op)
        .bench("HttpServer/scenario=tracing", |b| {
            let _tracer_provider = setup_tracer();
            measure(b, &rt, layered(handler), TARGET);
        })
        // Scenario 4: Tracing, request target with a query string that holds no
        // sensitive parameter (measures `url.query` and the scan for a key to redact)
        .bench("HttpServer/scenario=tracing-query", |b| {
            let _tracer_provider = setup_tracer();
            measure(b, &rt, layered(handler), TARGET_QUERY);
        })
        // Scenario 5: Tracing, request target with a query string that holds a
        // credential (measures redaction, which has to rewrite the value)
        .bench("HttpServer/scenario=tracing-query-redacted", |b| {
            let _tracer_provider = setup_tracer();
            measure(b, &rt, layered(handler), TARGET_QUERY_REDACTED);
        })
        // Scenario 6: Tracing with AlwaysOff sampler (global meter not set, so meter instruments are no-op)
        .bench("HttpServer/scenario=tracing-sampled-out", |b| {
            let _tracer_provider = setup_sampled_out_tracer();
            measure(b, &rt, layered(handler), TARGET);
        })
        // Scenario 7: Metrics only (tracer reset to NoopTracerProvider)
        .bench("HttpServer/scenario=metrics", |b| {
            noop_tracer();
            let (_meter_provider, _metric_exporter) = setup_meter();
            measure(b, &rt, layered(handler), TARGET);
        })
        // Scenario 8: Both tracing + metrics
        .bench("HttpServer/scenario=tracing+metrics", |b| {
            let _tracer_provider = setup_tracer();
            let (_meter_provider, _metric_exporter) = setup_meter();
            measure(b, &rt, layered(handler), TARGET);
        })
        // Scenario 9: Both tracing + metrics, with a 500 response. Comparing this
        // against scenario 8 isolates the cost of the `error.type` code path,
        // since both scenarios have the same signals active and differ only in
        // the response status.
        .bench("HttpServer/scenario=tracing+metrics-500", |b| {
            let _tracer_provider = setup_tracer();
            let (_meter_provider, _metric_exporter) = setup_meter();
            measure(b, &rt, layered(error_handler), TARGET);
        })
        .run();
}
