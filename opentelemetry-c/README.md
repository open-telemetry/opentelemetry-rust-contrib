# OpenTelemetry C

| Status    |              |
| --------- | ------------ |
| Stability | experimental |
| Owners    | @lalitb      |

A **Rust-backed C binding** for OpenTelemetry traces, delivered as one component split into
separate **API** and **SDK** libraries (plus an internal ABI crate). This split lets a C/C++
instrumentation library depend only on the API, while the application owns the SDK.

> ⚠️ **Experimental.** The C ABI is not yet stable and may change between `0.x` releases.

## Project structure

- **[api/](api/)** — package `opentelemetry-c-api`. The public C **API** facade (tracer
  providers, tracers, spans as opaque handles). Owns the single process-global provider slot
  with a no-op default. Depends only on the internal ABI crate — never on the SDK/OTLP.
  Instrumentation links **this library only**.
- **[sdk/](sdk/)** — package `opentelemetry-c-sdk`. The **SDK**: an OTLP HTTP/protobuf trace
  exporter and a batch span processor, composed through a pipeline of exporter/processor
  builders and installed via the SDK builder. Installing it registers an implementation into
  the API-owned global slot. Applications link **this plus the API**.
- **[abi/](abi/)** — package `opentelemetry-c-abi`. An **internal, Rust-only** rlib holding
  the shared `#[repr(C)]` types and the implementation vtable used across the API/SDK
  boundary. It has no exported C symbols and is not consumed directly by C.

## Consumption model

- **Instrumentation libraries** link only `libopentelemetry_c_api` (include `api.h`). Trace
  calls are safe no-ops until an application installs an SDK, then they dispatch to it.
- **Applications** link `libopentelemetry_c_api` **and** `libopentelemetry_c_sdk` (include
  `sdk.h` plus the pipeline headers); they build the trace pipeline, install it globally,
  flush, and shut down.

## Pipeline object model (SDK)

The SDK configures a trace pipeline from separate, composable objects that map to
OpenTelemetry concepts, rather than one monolithic builder:

1. Build a **trace exporter** (today: OTLP HTTP/protobuf — `otlp_trace_exporter.h`) →
   `otel_trace_exporter_t`.
2. Wrap it in a **span processor** (today: batch — `batch_span_processor.h`), which consumes
   the exporter → `otel_span_processor_t`.
3. Add the processor to the **SDK builder** (`sdk.h`), which consumes it, then build and
   install the SDK.

The generic `otel_trace_exporter_t` / `otel_span_processor_t` handles are opaque extension
points, so more exporter/processor kinds can be added later without breaking the C ABI. Each
`build`/`set_exporter`/`add_span_processor` transfers ownership on `OTEL_STATUS_OK`; see the
[sdk/README.md](sdk/README.md) and the pipeline headers for the exact rules.

See [api/README.md](api/README.md) and [sdk/README.md](sdk/README.md) for build/link
commands, ownership rules, and the runnable `sdk/examples/c-basic-traces` example.

## Hot-path performance contract

The C API/SDK is a **thin ABI boundary** over the Rust OpenTelemetry SDK. It must not add
runtime machinery on telemetry hot paths beyond required FFI marshalling and the Rust SDK's
own internals. This is a standing design invariant, not a one-off.

**Setup / cold path may allocate and use locks** — SDK/exporter/processor/resource builders,
OTLP and batch config, `otel_sdk_build`, `set_as_global`, `force_flush`/`shutdown`
coordination, and tests/examples.

**Span and tracer hot paths** — `otel_tracer_start_span`, the `otel_span_set_*` /
`otel_span_add_event` / `otel_span_set_status` / `otel_span_update_name` / `otel_span_end` /
`otel_span_destroy` calls and the SDK vtable functions they dispatch to — **must not** add, at
the C layer: new locks/`OnceLock`/registries/global maps, C-side batching or intermediate span
records, per-span clones of the provider/exporter/processor/config, exporter/processor/builder
access, environment-variable or config lookups, callbacks into user code, or extra
routing/dispatch beyond the single API→SDK implementation vtable.

**Accepted, required costs on hot paths:** opaque handle validation; API→SDK vtable
dispatch (normally one per operation; `otel_span_destroy` may call both `span_end` and
`span_free` to preserve best-effort end-before-free semantics); validating C
pointers/tags/lengths; converting borrowed C string/key/value views into SDK-owned values
(one owned allocation per key/value/name — borrowed C memory must not outlive the call);
allocating the real OTel span/tracer/handle objects; and the Rust SDK's own processing. The
single `RwLock` read and `Arc` clone used to retain the global provider happen **only when
resolving a tracer from the global provider** (`otel_tracer_provider_get_tracer`), never per
span/attribute/event — so cache the returned `otel_tracer_t` and reuse it. At the C
API/vtable layer, span operations take no global locks (the one-span-per-thread contract lets
the SDK vtable take `&mut` without C-layer synchronization). API hot-path entry points that
report failures clear the thread-local last-error slot at entry; that clear uses no global
lock and performs no heap allocation.

## Supported model

The shared-global model is validated on **Unix-like dynamic linking (Linux and macOS)** with
a single loaded `libopentelemetry_c_api`. **Windows dynamic linking is not yet verified**, and
static-linking the API into more than one artifact creates separate global provider slots (not
the shared-global model) — neither is claimed as supported. See the api/sdk READMEs for
details.

## License

Apache-2.0.
