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
- **[sdk/](sdk/)** — package `opentelemetry-c-sdk`. The **SDK**: OTLP HTTP/protobuf exporter,
  batch span processor, and lifecycle. Installing it registers an implementation into the
  API-owned global slot. Applications link **this plus the API**.
- **[abi/](abi/)** — package `opentelemetry-c-abi`. An **internal, Rust-only** rlib holding
  the shared `#[repr(C)]` types and the implementation vtable used across the API/SDK
  boundary. It has no exported C symbols and is not consumed directly by C.

## Consumption model

- **Instrumentation libraries** link only `libopentelemetry_c_api` (include `api.h`). Trace
  calls are safe no-ops until an application installs an SDK, then they dispatch to it.
- **Applications** link `libopentelemetry_c_api` **and** `libopentelemetry_c_sdk` (include
  `sdk.h`); they build the SDK, install it globally, flush, and shut down.

See [api/README.md](api/README.md) and [sdk/README.md](sdk/README.md) for build/link
commands, ownership rules, and the runnable `sdk/examples/c-basic-traces` example.

## Supported model

The shared-global model is validated on **Unix-like dynamic linking (Linux and macOS)** with
a single loaded `libopentelemetry_c_api`. **Windows dynamic linking is not yet verified**, and
static-linking the API into more than one artifact creates separate global provider slots (not
the shared-global model) — neither is claimed as supported. See the api/sdk READMEs for
details.

## License

Apache-2.0.
