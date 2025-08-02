# MSI Native Interface

This directory contains the C/C++ native interface files for MSI (Managed Service Identity) authentication support.

## Files

- **`wrapper.h`** - C header file defining the FFI interface between Rust and the native MSI library
- **`bridge.cpp`** - C++ implementation that bridges Rust calls to the XPlatLib MSI functionality

## Purpose

These files provide a C-compatible interface to the Microsoft XPlatLib MSI authentication library, allowing the Rust code to obtain MSI tokens for authentication with Azure services.

## Integration

The Rust FFI bindings in `../ffi.rs` reference these native files when the `msi_auth` feature is enabled. The build process (via `build.rs`) handles compilation and linking of these native components when MSI authentication support is available.
