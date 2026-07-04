# opentelemetry-c-sdk

[![Apache License][license-image]][license-url]

The **C SDK** of the Rust-backed OpenTelemetry C binding: an OTLP **HTTP/protobuf**
exporter and a batch span processor behind the `otel_sdk_*` C functions. Installing the
SDK registers it into the **API library's** global provider slot, so instrumentation that
links only [`opentelemetry-c-api`](../api) exports through it.

The exporter uses the blocking `reqwest` client, so the SDK owns all of its own threading
and **no user-managed async runtime is required**. HTTPS is supported via a selectable TLS
backend: `native-tls` (default, platform TLS) or `rustls-tls`.

> ⚠️ **Experimental.** The C ABI is not yet stable and may change between `0.x` releases.

## Linking model

Applications link **both** libraries and put both include directories on the search path
(this header includes the API's `common.h`/`trace.h`). Instrumentation libraries link only
the API. The SDK cdylib references the API cdylib's internal registration symbols, resolved
at load time — so the application must link the API alongside the SDK. This load-time
resolution is verified on **Unix-like dynamic linking (Linux, macOS)**; **Windows is not yet
verified** and needs an import-library follow-up (see the API README's Platform support).

```sh
cargo build --release -p opentelemetry-c-api -p opentelemetry-c-sdk

cc -std=c11 my_app.c \
   -I path/to/opentelemetry-c/api/include \
   -I path/to/opentelemetry-c/sdk/include \
   -L path/to/target/release -lopentelemetry_c_api -lopentelemetry_c_sdk \
   -Wl,-rpath,path/to/target/release -o my_app
```

Static linking may require additional native/system libraries from the Rust, TLS, and HTTP
dependencies; discover them with
`cargo rustc --release --lib -- --print native-static-libs`.

### Library lifetime

The shared-global model requires **dynamic linking with exactly one loaded
`libopentelemetry_c_api`** (see the API README).

Once `otel_sdk_set_as_global` succeeds, it publishes this crate's `'static` implementation
vtable and an SDK-owned provider object into the API's global slot. **`otel_sdk_shutdown`
and `otel_sdk_destroy` do not clear that slot** — they stop and free the `otel_sdk_t`
handle, but the slot keeps referencing this library's vtable/provider. The slot is cleared
only when **another provider replaces it** (a subsequent `otel_sdk_set_as_global` /
registration).

Therefore, after a successful global install, **`libopentelemetry_c_sdk` must remain loaded
until process exit, or until another provider replaces the global slot** — shutting down and
destroying the SDK does **not** make unloading it safe. Any live SDK-backed handles (tracer
provider, tracer, span obtained after `set_as_global`) must also be destroyed before unload.
Statically linking the API into multiple artifacts creates separate global slots and is
**not** the shared-global model.

A ready-to-run example that links both libraries is in
[`examples/c-basic-traces/`](examples/c-basic-traces) (`make run`).

## Minimal C usage

### Instrumentation library: API only

Instrumentation libraries include only `api.h` and link only `libopentelemetry_c_api`. Calls are
safe no-ops until the application installs an SDK.

```c
#include <opentelemetry_c/api.h>

void do_work(void) {
    otel_tracer_provider_t* provider = otel_global_tracer_provider();
    otel_tracer_t* tracer = otel_tracer_provider_get_tracer(
        provider, otel_cstr("my-instrumentation"), otel_cstr("0.1.0"),
        otel_string_view_empty());

    otel_span_t* span = otel_tracer_start_span(tracer, otel_cstr("work"), NULL);
    otel_span_set_string_attribute(span, otel_cstr("example.key"), otel_cstr("value"));
    otel_span_set_ok(span);
    otel_span_end(span);

    otel_span_destroy(span);
    otel_tracer_destroy(tracer);
    otel_tracer_provider_destroy(provider);
}
```

### Application: SDK setup

Applications link both `libopentelemetry_c_api` and `libopentelemetry_c_sdk`. They configure the
trace pipeline and install it globally before instrumentation runs.

```c
#include <opentelemetry_c/api.h>
#include <opentelemetry_c/batch_span_processor.h>
#include <opentelemetry_c/otlp_trace_exporter.h>
#include <opentelemetry_c/sdk.h>

int main(void) {
    otel_otlp_trace_exporter_builder_t* eb = otel_otlp_trace_exporter_builder_new();
    otel_otlp_trace_exporter_builder_set_endpoint(
        eb, otel_cstr("http://localhost:4318/v1/traces"));

    otel_trace_exporter_t* exporter = NULL;
    otel_otlp_trace_exporter_builder_build(eb, &exporter);
    otel_otlp_trace_exporter_builder_destroy(eb);

    otel_batch_span_processor_builder_t* pb = otel_batch_span_processor_builder_new();
    otel_batch_span_processor_builder_set_exporter(pb, exporter);

    otel_span_processor_t* processor = NULL;
    otel_batch_span_processor_builder_build(pb, &processor);
    otel_batch_span_processor_builder_destroy(pb);

    otel_sdk_builder_t* sb = otel_sdk_builder_new();
    otel_sdk_builder_set_service_name(sb, otel_cstr("example-service"));
    otel_sdk_builder_add_span_processor(sb, processor);

    otel_sdk_t* sdk = NULL;
    otel_sdk_build(sb, &sdk);
    otel_sdk_builder_destroy(sb);
    otel_sdk_set_as_global(sdk);

    do_work();

    otel_sdk_force_flush(sdk, 5000);
    otel_sdk_shutdown(sdk, 5000);
    otel_sdk_destroy(sdk);
    return 0;
}
```

The complete buildable version with error handling and a `Makefile` is
[`examples/c-basic-traces`](examples/c-basic-traces).

## Pipeline object model

The SDK builds a trace pipeline from separate, composable objects that map to OpenTelemetry
concepts, so the SDK builder is not coupled to any one exporter or processor:

```
OTLP exporter builder ──build──▶ otel_trace_exporter_t
                                        │ set_exporter (ownership transfers)
                                        ▼
batch span processor builder ─build─▶ otel_span_processor_t
                                        │ add_span_processor (ownership transfers)
                                        ▼
                 SDK builder ──build──▶ otel_sdk_t ──set_as_global──▶ global provider
```

Only the **OTLP HTTP/protobuf trace exporter** and the **batch span processor** are
implemented today. The generic `otel_trace_exporter_t` / `otel_span_processor_t` handles are
opaque extension points: internally each wraps an enum (`TraceExporterImpl` implementing
`SpanExporter`, `SpanProcessorImpl` implementing `SpanProcessor`), so another exporter or
processor kind is a new variant plus a builder — no change to the C ABI, the generic handles,
or the SDK builder's storage. No custom-callback exporter is provided yet.

### Cargo features (optional OTLP)

The **SDK core** — the builder, `SdkTracerProvider`, the batch span processor, and the generic
exporter/processor handles — is a separate concern from any exporter implementation. The OTLP
HTTP/protobuf exporter is an **optional** exporter, enabled by default:

| Feature | Default | Effect |
| --- | --- | --- |
| `otlp` | ✅ (via TLS features) | Compile in the OTLP HTTP/protobuf exporter (`opentelemetry-otlp`, `reqwest`). |
| `native-tls` | ✅ | Implies `otlp`; OTLP HTTPS via the OS TLS stack (`reqwest/native-tls`). |
| `rustls-tls` | ❌ | Implies `otlp`; OTLP HTTPS via rustls (`reqwest/rustls`). |

Building with `--no-default-features` produces the SDK core **without** `opentelemetry-otlp`,
`reqwest`, or any TLS backend. The `otel_otlp_trace_exporter_builder_*` symbols remain (the C
ABI is identical across feature sets), but `otel_otlp_trace_exporter_builder_build` returns
`OTEL_STATUS_INVALID_CONFIG` with a last-error explaining the `otlp` feature is disabled.
Enabling `otlp` without a TLS feature builds an HTTP-only OTLP exporter (no HTTPS).

### Ownership transfer rules

- A `build(builder, &out)` call creates a new owned object; the builder stays owned by the
  caller (destroy it when done).
- `otel_batch_span_processor_builder_set_exporter` transfers the exporter into the processor
  builder **on `OTEL_STATUS_OK`**; on failure the caller still owns it.
- `otel_sdk_builder_add_span_processor` transfers the processor into the SDK builder **on
  `OTEL_STATUS_OK`**; on failure the caller still owns it.
- After a successful transfer, do **not** destroy the transferred handle (its destroy becomes
  a safe no-op).
- Destroying a builder frees any transferred children it still owns (i.e. that a later
  `build` did not consume). All `*_destroy` functions are NULL-safe and must not race with
  other use of the same handle.

## Headers

- [`include/opentelemetry_c/sdk.h`](include/opentelemetry_c/sdk.h) — SDK builder, resource
  config, `add_span_processor`, build, and lifecycle (`set_as_global`, `get_tracer_provider`,
  `force_flush`, `shutdown`, `destroy`).
- [`otlp_trace_exporter.h`](include/opentelemetry_c/otlp_trace_exporter.h) — OTLP HTTP/protobuf
  exporter builder (endpoint / header / timeout).
- [`batch_span_processor.h`](include/opentelemetry_c/batch_span_processor.h) — batch processor
  builder (exporter + bounded queue/delay/batch settings).
- [`trace_exporter.h`](include/opentelemetry_c/trace_exporter.h) /
  [`span_processor.h`](include/opentelemetry_c/span_processor.h) — the generic opaque handles.

## Behavior & guarantees

- Application owns the pipeline + SDK lifecycle: build exporter → build processor → build SDK
  → `set_as_global` → (instrumentation emits spans via the API) → `force_flush` → `shutdown`.
  Shutdown runs at most once.
- Batch queue / export-batch sizes from C are bounded; oversized values are rejected with
  `OTEL_STATUS_INVALID_ARGUMENT`, `0` selects the SDK default.
- All entry points are panic-safe. Runtime export failures never crash the process.
- The SDK library never re-exports the API/trace/common functions, so linking both
  libraries produces no duplicate symbols.

## Tests

`cargo test -p opentelemetry-c-sdk --all-features` covers the vtable trace behavior
(parent/child semantics), global registration, batch bounds, and force-flush cleanup. The
`cross_artifact` integration test compiles a C program, links it against **both** built
cdylibs, and confirms API-only spans (after SDK install) export through the SDK to a mock
collector — proving the shared global provider.

Because `cargo test` does not emit cdylib artifacts, build them first:

```sh
cargo build -p opentelemetry-c-api -p opentelemetry-c-sdk --all-features
cargo test -p opentelemetry-c-sdk --test cross_artifact --all-features
```

The repository's `scripts/test.sh` performs this build step automatically. The test
self-skips only as a local developer convenience (missing C compiler or unbuilt cdylibs);
under `CI` it fails hard instead, so the proof can never silently no-op. Verified on
Unix-like dynamic linking (Linux, macOS); see the API README for the Windows status.

## License

Apache-2.0.

[license-image]: https://img.shields.io/badge/license-Apache_2.0-green.svg
[license-url]: https://github.com/open-telemetry/opentelemetry-rust-contrib/blob/main/LICENSE
