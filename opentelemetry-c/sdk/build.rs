//! Build script for `opentelemetry-c-sdk`.
//!
//! The SDK cdylib references the API cdylib's internal registration symbols
//! (`otel_api_register_global_provider`, `otel_api_provider_new`, `otel_api_set_last_error`,
//! `otel_api_clear_last_error`). Those are resolved at **load time** against
//! `libopentelemetry_c_api` — which the application links alongside this library.
//!
//! On macOS the dynamic linker rejects undefined symbols in a dylib unless told otherwise,
//! so allow dynamic lookup for the cdylib target. On Linux, undefined symbols in a shared
//! object are permitted by default (resolved at load time), so no flag is needed. The flag
//! applies only to the `cdylib` target, not to the rlib used by this crate's Rust tests.

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "macos" || target_os == "ios" {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-undefined,dynamic_lookup");
    }
}
