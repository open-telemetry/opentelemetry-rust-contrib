# opentelemetry-c-sdk

[![Apache License][license-image]][license-url]

The **C SDK** of the Rust-backed OpenTelemetry C binding: an OTLP **HTTP/protobuf**
exporter and a batch span processor behind the `otel_sdk_*` C functions. Installing the
SDK registers it into the **API library's** global provider slot, so instrumentation that
links only [`opentelemetry-c-api`](../opentelemetry-c-api) exports through it.

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
   -I path/to/opentelemetry-c-api/include \
   -I path/to/opentelemetry-c-sdk/include \
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

## Header

[`include/opentelemetry_c/sdk.h`](include/opentelemetry_c/sdk.h) — SDK builder and lifecycle
(`otel_sdk_builder_*`, `otel_sdk_build`, `otel_sdk_set_as_global`,
`otel_sdk_get_tracer_provider`, `otel_sdk_force_flush`, `otel_sdk_shutdown`,
`otel_sdk_destroy`).

## Behavior & guarantees

- Application owns SDK lifecycle: build → `set_as_global` → (instrumentation emits spans via
  the API) → `force_flush` → `shutdown`. Shutdown runs at most once.
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
