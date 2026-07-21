# opentelemetry-c-api

[![Apache License][license-image]][license-url]

The **C API facade** of the Rust-backed OpenTelemetry C binding. It exposes the public
trace API (tracer providers, tracers, spans) as opaque C handles, **owns the single
process-global provider slot**, and ships a **no-op default** so API-only instrumentation
is safe with or without an SDK.

This crate depends only on an internal ABI-types crate — never on `opentelemetry_sdk`,
`opentelemetry-otlp`, or `reqwest`.

> ⚠️ **Experimental.** The C ABI is not yet stable and may change between `0.x` releases.

## The API/SDK split

`opentelemetry-c` is split into two linkable artifacts:

| Library | Who links it | Contains |
| --- | --- | --- |
| **`libopentelemetry_c_api`** (this crate) | instrumentation **and** applications | trace API, global provider slot, no-op default |
| **`libopentelemetry_c_sdk`** | applications only | OTLP exporter, batch processor, SDK lifecycle |

- **Instrumentation libraries** link **only** `libopentelemetry_c_api`. Their trace calls
  are safe no-ops until an application installs an SDK, then they dispatch to it.
- **Applications** link **both** libraries. Installing the SDK
  (`otel_sdk_set_as_global`) registers it into *this* library's global provider slot
  (across the C ABI via the internal `otel_api_register_global_provider`), so it becomes
  visible to all API-only instrumentation.

There is exactly **one** global provider slot in the process — owned here — so no
duplicate global state exists across the two libraries.

### Linking & library lifetime (important)

The shared-global model is only guaranteed under **dynamic linking with exactly one loaded
`libopentelemetry_c_api`**:

- **Dynamic linking (supported model).** Instrumentation and the application resolve the
  same `libopentelemetry_c_api` at load time, so they share the one global provider slot.
- **Static linking into multiple artifacts is *not* the shared-global model.** If
  `opentelemetry-c-api` is statically linked into more than one artifact (e.g. an
  instrumentation library *and* the application each statically embed it), each copy gets
  its **own** global provider slot and its own no-op default. An SDK installed into one slot
  is invisible to the other. Link the API as a single shared library so all callers observe
  one slot.
- **Keep the SDK loaded for the process lifetime after a global install.** Installing an
  SDK (`otel_sdk_set_as_global`) publishes the SDK's `'static` implementation vtable and an
  SDK-owned provider object into this library's global slot. **`otel_sdk_shutdown` and
  `otel_sdk_destroy` do *not* clear that slot** — they stop and free the `otel_sdk_t` handle
  but leave the slot pointing at the SDK's vtable/provider. The slot is only cleared when
  **another provider replaces it** (a later `otel_sdk_set_as_global` / registration).
  Therefore, once `otel_sdk_set_as_global` succeeds, **`libopentelemetry_c_sdk` must remain
  loaded until process exit, or until another provider replaces the global slot** — shutting
  down and destroying the SDK does **not** make unloading it safe. (Any live SDK-backed
  tracer/span handles must also be destroyed before unload.)

## Headers

Under [`include/opentelemetry_c/`](include/opentelemetry_c):

- `common.h` — status codes, string views, typed attributes, version/error queries.
- `trace.h` — tracer provider, tracer, and span handles.
- `api.h` — umbrella (`common.h` + `trace.h`).

### Optional convenience helpers

Purely optional `static inline` (C99+/C++) wrappers over the raw API — **header-only, no ABI
symbols, no allocation or copy** (the status shorthands just perform the one
`otel_span_set_status()` call they wrap). String views are passed through borrowed, so the
referenced bytes must stay valid until the wrapped call returns. The public headers remain the
full reference:

- `otel_kv_string` / `otel_kv_bool` / `otel_kv_int64` / `otel_kv_double` — build a typed
  `otel_key_value_t` by value, e.g. for `otel_span_add_event()` attribute arrays.
- `otel_span_set_ok` / `otel_span_set_error` — optional status shorthands over
  `otel_span_set_status()`.
- `otel_cstr` / `otel_string_view_empty` — build a string view from a C string / an empty view.

## Building & linking

```sh
cargo build --release -p opentelemetry-c-api
```

This emits, under `target/release/`:

- a **shared library** (cdylib: `.so` / `.dylib`) — the artifact used by the supported
  dynamic-linking model;
- a **static library** (staticlib: `.a`) — see the static-linking caveat below;
- an `rlib` for Rust tests/internal use.

An **instrumentation library** compiles against the headers and links only the API:

```sh
cc -std=c11 my_instr.c \
   -I path/to/opentelemetry-c/api/include \
   -L path/to/target/release -lopentelemetry_c_api \
   -Wl,-rpath,path/to/target/release -o my_instr
```

Applications additionally link `libopentelemetry_c_sdk` — see that crate's README and the
`c-basic-traces` example.

**Static-linking caveat.** The static library is emitted, but statically linking the API into
**more than one artifact** (e.g. an instrumentation library *and* the application) gives each
copy its own global provider slot, so an installed SDK is invisible across them (see *Linking
& library lifetime* above). The guaranteed shared-global model is dynamic linking with a
single loaded `libopentelemetry_c_api`; static linking is only equivalent in a single binary
that embeds exactly one copy of the API.

## Platform support

The dynamic API/SDK split (instrumentation links the API only; the SDK registers into the
API-owned global slot) is verified on **Unix-like dynamic linking — Linux and macOS**. The
cross-artifact proof test runs there.

**Windows is not yet verified.** The SDK cdylib references the API cdylib's `otel_api_*`
symbols, which on Windows requires linking against the API's generated import library
(`.dll.lib`) rather than the load-time dynamic-lookup resolution used on Unix. Producing and
wiring that import library is follow-up work; until then, treat Windows dynamic linking as
unsupported. (Rust `cargo check`/`clippy` of the crates still succeed on Windows because they
do not link the cdylibs.)

## Ownership & safety

- Every handle-returning function transfers ownership; release with the matching
  `*_destroy`. Pass only NULL or a **live handle of the exact expected type**; the magic
  check is a best-effort diagnostic, not a safety net (use-after-destroy is UB).
- Strings are borrowed `otel_string_view_t` values, copied before return.
- All entry points are panic-safe (a Rust panic is caught, never unwound into C).
- SDK/provider/tracer handles are safe to share across threads; a single span handle is
  not (one span per thread). `*_destroy` must not race with other calls on the same handle.

## License

Apache-2.0.

[license-image]: https://img.shields.io/badge/license-Apache_2.0-green.svg
[license-url]: https://github.com/open-telemetry/opentelemetry-rust-contrib/blob/main/LICENSE
