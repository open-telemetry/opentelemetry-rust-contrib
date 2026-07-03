# opentelemetry-c-abi

**Internal** crate (`publish = false`). Defines the shared `#[repr(C)]` value types and the
internal implementation vtable (`OtelImplVtable`) used across the `opentelemetry-c` API/SDK
C-ABI boundary, plus pure string/validation helpers.

It contains **no global state and no `#[no_mangle]` exports**, so it is linked statically
into both the [`opentelemetry-c-api`](../opentelemetry-c-api) and
[`opentelemetry-c-sdk`](../opentelemetry-c-sdk) cdylibs without introducing duplicate
exported symbols or duplicate global provider state. Not intended for direct use.

Licensed under Apache-2.0.
