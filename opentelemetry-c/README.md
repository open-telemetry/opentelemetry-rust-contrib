# OpenTelemetry C API/SDK (Rust-backed)

[![Apache License][license-image]][license-url]

A **C API and SDK for [OpenTelemetry](https://opentelemetry.io)**, implemented in Rust.
This crate exposes an experimental C ABI over the Rust OpenTelemetry SDK so that C (and C++)
applications can create traces and export them via OTLP without writing any Rust.

This is a contrib component of
[`opentelemetry-rust-contrib`](https://github.com/open-telemetry/opentelemetry-rust-contrib).

> ⚠️ **Experimental.** This crate is experimental and the C ABI is **not yet stable**.
> Symbols, struct layouts, and enum values may change between `0.x` releases. Pin an
> exact version and re-test when upgrading. It is not yet recommended for production
> pending maintainer review and ABI stabilization.

## Status

| Signal  | Status                     |
| ------- | -------------------------- |
| Traces  | ✅ Implemented (this crate) |
| Metrics | ⛔ Not yet (planned)        |
| Logs    | ⛔ Not yet (planned)        |

The headers are designed so metrics and logs can be added later (as `metrics.h` /
`logs.h` with their own handle types) **without breaking the traces ABI**.

## Why Rust-backed?

The public surface is plain C — opaque handles, status codes, and POD structs. All of
the real work (batching, retrying, OTLP encoding, HTTP transport, threading) is done by
the mature Rust OpenTelemetry SDK behind the ABI. C callers never see Rust types,
traits, builders, `Arc`, ownership rules, or async runtimes.

The default OTLP exporter uses **HTTP/protobuf with a blocking HTTP client**, so the
SDK owns all of its own threads (a dedicated batch-processor thread and the HTTP
client). **No user-managed async runtime is required.**

## Supported platforms

Built and tested on Linux and macOS with `gcc`/`clang`. Windows (MSVC) is expected to
work via the same Cargo `cdylib`/`staticlib` outputs but is not yet part of routine
testing. 64-bit targets are the primary focus; the headers include `_Static_assert`
layout checks on 64-bit, C11+ compilers.

## Building the library

You need a [Rust toolchain](https://rustup.rs) (edition 2021, MSRV 1.75). From this
directory:

```sh
# Debug build
cargo build

# Optimized build (recommended for distribution)
cargo build --release
```

Cargo produces, under `../target/<profile>/`:

- `libopentelemetry_c.so` / `.dylib` / `opentelemetry_c.dll` — C-linkable **shared**
  library (`cdylib`).
- `libopentelemetry_c.a` (or the platform equivalent) — **static** library (`staticlib`).
- `libopentelemetry_c.rlib` — Rust **rlib**, used only by this crate's own tests and
  internal Rust builds; C/C++ consumers do not use it.

C and C++ consumers use two things: the public headers from
[`include/opentelemetry_c/`](include/opentelemetry_c), and the `cdylib` (shared) or
`staticlib` (static) library from the Cargo target output above.

- `common.h` — status codes, booleans, string views, typed attributes, version/error.
- `trace.h` — tracer provider, tracer, and span handles.
- `sdk.h` — SDK builder and lifecycle.
- `api.h` — umbrella header that includes all of the above.

### TLS backend (HTTPS export)

OTLP over HTTPS is supported. The TLS backend is selected with cargo features:

- **`native-tls`** *(default)* — uses the operating system's TLS stack (OpenSSL on
  Linux, Secure Transport on macOS, SChannel on Windows). No crypto is bundled and the
  rustls dependency tree is not pulled in. On Linux this requires the OpenSSL
  development headers at build time.
- **`rustls-tls`** — uses [rustls](https://github.com/rustls/rustls) (pure-Rust TLS)
  with a bundled crypto provider, avoiding a system OpenSSL dependency:

  ```sh
  cargo build --release --no-default-features --features rustls-tls
  ```

Plain-HTTP endpoints (e.g. a local collector on `:4318`) work regardless of the TLS
feature selected.

## Linking from C

### Against the shared library (simplest)

```sh
cc -std=c11 my_app.c \
   -I path/to/opentelemetry-c/include \
   -L path/to/target/release -lopentelemetry_c \
   -Wl,-rpath,path/to/target/release \
   -o my_app
```

At **run time** the dynamic loader must be able to find the shared library. The
`-Wl,-rpath` above bakes a search path into the executable; alternatively:

- **Linux:** set `LD_LIBRARY_PATH` to the library's directory, or install it to a standard
  location (e.g. `/usr/local/lib`, then `ldconfig`).
- **macOS:** set `DYLD_LIBRARY_PATH`, or use an `@rpath` / install-name based layout.
- **Windows:** place `opentelemetry_c.dll` next to the executable or on the `PATH`.

### Against the static library

Static linking pulls the Rust runtime, TLS, and HTTP stacks into your binary, so it may
require **additional native/system libraries** from those dependencies at link time. The
exact set is platform- and feature-dependent — print it for this crate with:

```sh
cargo rustc --release --lib -- --print native-static-libs
```

Add the native libraries it reports to your link line. The example below is
**illustrative and platform-dependent** (Linux); take the authoritative list from the
command above rather than hard-coding it:

```sh
# Linux, static — illustrative; confirm the trailing libraries with --print native-static-libs
cc -std=c11 my_app.c \
   -I path/to/opentelemetry-c/include \
   path/to/target/release/libopentelemetry_c.a \
   -lpthread -ldl -lm \
   -o my_app
```

A ready-to-run example with a `Makefile` (covering both Linux and macOS) is in
[`examples/c-basic-traces/`](examples/c-basic-traces).

## Using from C++

C++ consumes the **same headers** — every declaration in `include/opentelemetry_c/` is
already wrapped in `extern "C"`, so you can `#include <opentelemetry_c/api.h>` directly
from C++ and link the same shared or static library described above.

If you want RAII ownership, add **thin local wrappers** in your application or
instrumentation library that call the matching `*_destroy` from a destructor (for example
a `std::unique_ptr` with a custom deleter per handle type). This crate intentionally does
**not** ship a C++ wrapper library; keeping the public surface plain C keeps it language-
and toolchain-agnostic and lets each consumer choose its own idioms.

## Minimal C example

```c
#include <opentelemetry_c/api.h>
#include <stdio.h>

int main(void) {
    /* 1. Build the SDK. */
    otel_sdk_builder_t *builder = otel_sdk_builder_new();
    otel_sdk_builder_set_service_name(builder, otel_cstr("my-service"));
    otel_sdk_builder_set_otlp_endpoint(builder, otel_cstr("http://localhost:4318/v1/traces"));

    otel_sdk_t *sdk = NULL;
    if (otel_sdk_build(builder, &sdk) != OTEL_STATUS_OK) {
        otel_string_view_t err = otel_last_error_message();
        fprintf(stderr, "build failed: %.*s\n", (int)err.len, err.ptr);
        otel_sdk_builder_destroy(builder);
        return 1;
    }
    otel_sdk_builder_destroy(builder);     /* builder is only read by build() */

    /* 2. Install globally and get a tracer. */
    otel_sdk_set_as_global(sdk);
    otel_tracer_provider_t *provider = otel_global_tracer_provider();
    otel_tracer_t *tracer = otel_tracer_provider_get_tracer(
        provider, otel_cstr("my-service"), otel_cstr("1.0.0"), otel_string_view_empty());

    /* 3. Create a span, annotate it, end it. */
    otel_span_t *span = otel_tracer_start_span(tracer, otel_cstr("do-work"), NULL);
    otel_span_set_string_attribute(span, otel_cstr("component"), otel_cstr("demo"));
    otel_span_set_int64_attribute(span, otel_cstr("items"), 3);
    otel_span_set_status(span, OTEL_SPAN_STATUS_OK, otel_string_view_empty());
    otel_span_end(span);
    otel_span_destroy(span);

    otel_tracer_destroy(tracer);
    otel_tracer_provider_destroy(provider);

    /* 4. Flush and shut down before exit. */
    otel_sdk_force_flush(sdk, 5000);
    otel_sdk_shutdown(sdk, 5000);
    otel_sdk_destroy(sdk);
    return 0;
}
```

## Running against an OTLP collector

Point the SDK at any OTLP/HTTP endpoint. For local development, an
[OpenTelemetry Collector](https://opentelemetry.io/docs/collector/) with the `debug`
exporter is the quickest way to see spans:

```yaml
# collector-config.yaml
receivers:
  otlp:
    protocols:
      http:
        endpoint: 0.0.0.0:4318
exporters:
  debug:
    verbosity: detailed
service:
  pipelines:
    traces:
      receivers: [otlp]
      exporters: [debug]
```

```sh
docker run --rm -p 4318:4318 \
  -v "$(pwd)/collector-config.yaml:/etc/otelcol/config.yaml" \
  otel/opentelemetry-collector:latest
```

The endpoint can also be supplied via the standard environment variables
`OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` or `OTEL_EXPORTER_OTLP_ENDPOINT`; programmatic
configuration (`otel_sdk_builder_set_otlp_endpoint`) takes precedence.

## Ownership rules

- Every function that **returns a handle** (`*_new`, `*_build` out-param,
  `*_get_tracer`, `*_start_span`, `*_tracer_provider`) transfers ownership to the
  caller. The caller **must** release it with the matching `*_destroy`.
- Every `*_destroy` accepts `NULL` as a safe no-op. Do **not** destroy the same
  non-NULL handle twice (this is a use-after-free, exactly like C `free`).
- **You must pass only `NULL` or a live handle of the exact expected type** returned by
  this library. Handles carry a per-type magic number, but it is a **best-effort
  diagnostic, not a safety net**: `NULL` is rejected up front, but the magic is read only
  *after* the pointer is dereferenced as the expected type, so it cannot be relied upon to
  catch a wrong handle type, a freed/already-destroyed handle, or a foreign pointer (all
  undefined behavior to pass). Passing the wrong handle type, using a handle after
  `destroy`, double-destroying, or racing `destroy` with another call on the same handle
  is undefined behavior.
- **All strings are borrowed for the duration of the call only.** They are passed as
  `otel_string_view_t` (pointer + length, UTF-8, not required to be NUL-terminated).
  The library copies whatever it needs to retain before returning, so the caller may
  free or reuse the bytes immediately afterward.
- C never frees Rust-allocated memory directly; Rust never takes ownership of C memory.
- Getting a tracer provider from an SDK (`otel_sdk_get_tracer_provider`) returns an
  **independent** handle; destroying it does not affect the SDK, and it remains valid
  after the SDK handle is destroyed (though telemetry stops once the SDK is shut down).

## Consuming from an instrumentation library

When integrating this API into a reusable C/C++ instrumentation library (as opposed to an
application), keep SDK lifecycle ownership with the application:

- The **application** owns SDK setup and teardown: `otel_sdk_build`,
  `otel_sdk_set_as_global`, `otel_sdk_force_flush`, and `otel_sdk_shutdown`. A library
  should **not** flush or shut down the SDK unless it also created and installed it.
- An **instrumentation library** should usually accept or create only what it needs —
  typically an `otel_tracer_t*` (from a provided provider or the global one) and the spans
  it starts — and release exactly the handles it owns.
- **Document ownership at your API boundary:** for every handle you accept or return,
  state whether the caller or the library is responsible for the matching `*_destroy`.

## Thread-safety rules

Each handle type has its own contract; there is no blanket guarantee.

- **SDK handles (`otel_sdk_t`)** are safe to use concurrently from multiple threads:
  `otel_sdk_set_as_global`, `otel_sdk_force_flush`, `otel_sdk_shutdown`, and
  `otel_sdk_get_tracer_provider` may all run at the same time on one handle. A concurrent
  `set_as_global` and `shutdown` may linearize in either order; once shutdown is observed,
  `set_as_global` returns `OTEL_STATUS_ALREADY_SHUTDOWN`.
- **Tracer providers and tracers** are safe to share and use concurrently.
- A **single span handle must not be used concurrently** from multiple threads. Use
  one span per thread, or synchronize access externally. Distinct spans may be used on
  different threads simultaneously.
- **SDK builder handles (`otel_sdk_builder_t`) are not thread-safe** — confine a builder
  to a single thread. (Build the SDK, then share the resulting `otel_sdk_t`.)
- **`*_destroy` must not race** with any other call on the same handle. Ensure all other
  operations on a handle have returned before you destroy it. (Handles are not
  reference-counted; destruction is not synchronized against concurrent use.)
- `otel_last_error_message()` returns a **thread-local** message describing the calling
  thread's most recent failure. The returned view is valid only until the next
  OpenTelemetry C call on the same thread.
- All entry points are panic-safe: a Rust panic is caught at the boundary and reported
  as `OTEL_STATUS_INTERNAL_ERROR` (or a NULL handle), never unwound into C.

## Threading model & async

The SDK owns all of its own threads — a dedicated batch-processor thread plus the
blocking HTTP client — so **no user-managed async runtime (e.g. Tokio) is required**.
Span export happens on the batch-processor thread, not on the caller's thread.

The library **never calls back into C**: there are no function-pointer callbacks in the
API, so you do not need to worry about C code being invoked from an SDK-owned thread.

A timed `otel_sdk_force_flush()` runs the flush on a short-lived helper thread so the
timeout can be honored even if the exporter stalls; **at most one such helper exists at a
time** (a concurrent timed flush returns `OTEL_STATUS_TIMEOUT` rather than spawning
another). A blocking flush (`timeout_millis == 0`) uses the calling thread and spawns no
helper.

## Shutdown / force-flush requirement

Span export is **asynchronous and batched**. You should call `otel_sdk_shutdown()`
(which flushes and stops the pipeline) before your process exits, otherwise buffered
spans may be lost. `otel_sdk_force_flush()` can be used at any time to flush eagerly.

- `otel_sdk_force_flush(sdk, timeout_millis)` — flush now; `timeout_millis == 0` blocks
  until done, otherwise returns `OTEL_STATUS_TIMEOUT` if it does not finish in time
  (the flush continues in the background), or `OTEL_STATUS_INTERNAL_ERROR` if a helper
  thread cannot be spawned.
- `otel_sdk_shutdown(sdk, timeout_millis)` — flush and stop. The underlying shutdown runs
  **at most once**: the first call performs it; concurrent or subsequent calls return
  `OTEL_STATUS_ALREADY_SHUTDOWN` and are otherwise harmless. `timeout_millis == 0` uses
  the SDK default (5s).

Destroying an SDK that was never explicitly shut down triggers a best-effort shutdown,
but you cannot observe its result or bound its time — prefer explicit shutdown.

## Error handling

Fallible functions return an `otel_status_t`; `OTEL_STATUS_OK` (0) is success. On
failure, call `otel_last_error_message()` for a human-readable diagnostic. **Runtime
export failures never crash the process** — they surface as a non-OK status from
`force_flush`/`shutdown` and are logged by the SDK. SDK construction fails fast with a
status for invalid configuration.

Configuration values coming from C are bounded: the batch **max queue size** and **max
export batch size** are capped at internal maximums, and an oversized non-zero value is
rejected with `OTEL_STATUS_INVALID_ARGUMENT` (never silently clamped) so it cannot drive a
large up-front allocation. `0` still selects the SDK/spec default, and the effective export
batch size is additionally capped by the SDK at the max queue size.

## Known limitations

- Traces only. Metrics and logs are not implemented yet.
- OTLP over **HTTP/protobuf** only (blocking client). gRPC is not exposed.
- No sampler / span-limit configuration from C yet (SDK defaults are used).
- Span links (other than a single parent) are not exposed yet.
- Context propagation across process boundaries (extract/inject) is not exposed yet;
  parenting is done explicitly by passing a parent span handle.
- The C ABI is experimental and unstable (see the note at the top).

## Packaging status

Formal packaging integrations — `pkg-config` files, a CMake package config, vcpkg/Conan
recipes, distro packages, and install rules — are **intentionally not included yet**.
Today, consumers use the headers in `include/opentelemetry_c/` and the `cdylib` /
`staticlib` from Cargo's target output directly (see [Linking from C](#linking-from-c) and
[Using from C++](#using-from-c)). These integrations can be added later once distribution
and install-layout expectations are agreed; until then, do not assume a stable installed
file layout.

## Regenerating headers with cbindgen (optional)

The headers in `include/opentelemetry_c/` are **hand-maintained** and are the source of
truth; a normal build does **not** require [`cbindgen`](https://github.com/mozilla/cbindgen).
A [`cbindgen.toml`](cbindgen.toml) is provided to cross-check the hand-written headers
against the Rust source during development:

```sh
cargo install cbindgen
cbindgen --config cbindgen.toml --crate opentelemetry-c --output /tmp/opentelemetry_c_generated.h
```

Treat cbindgen output as a review aid, not a drop-in replacement for the curated,
split, and documented headers shipped here.

## License

Licensed under [Apache License, Version 2.0][license-url].

[license-image]: https://img.shields.io/badge/license-Apache_2.0-green.svg
[license-url]: https://github.com/open-telemetry/opentelemetry-rust-contrib/blob/main/LICENSE
