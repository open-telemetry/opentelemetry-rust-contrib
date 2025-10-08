# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository Overview

This is the OpenTelemetry Rust Contrib repository, containing community-supported vendor integrations and utilities that extend the core [OpenTelemetry Rust](https://github.com/open-telemetry/opentelemetry-rust) implementation. This is a Cargo workspace with multiple independent crates.

**Minimum Rust version:** 1.75 (tracks latest stable and 3 prior minor versions)

## Common Commands

### Build
```bash
cargo build --workspace --all-features
```

### Testing
```bash
# Run all tests with all features
./scripts/test.sh

# Or manually:
cargo test --workspace --all-features --tests
cargo test --workspace --all-features --doc
```

### Linting
```bash
# Run all lints
./scripts/lint.sh

# Or manually:
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -Dwarnings
```

### Pre-commit checks
```bash
./precommit.sh
```
This runs: `cargo update && cargo fmt --all && ./scripts/lint.sh && ./scripts/test.sh`

### Single crate operations
```bash
# Test a specific crate
cargo test -p <crate-name> --all-features

# Build a specific crate
cargo build -p <crate-name> --all-features

# Lint a specific crate with specific features
cargo clippy --manifest-path=<crate-path>/Cargo.toml --all-targets --features "<features>" -- -Dwarnings
```

### Documentation
```bash
cargo doc --no-deps --all-features
```

### Benchmarks
```bash
cargo bench
```

## Workspace Structure

The repository is organized as a Cargo workspace with the following major crates:

### Vendor Exporters & Propagators
- **opentelemetry-aws**: AWS XRay propagator and ID generator
- **opentelemetry-datadog**: Datadog exporter (supports multiple HTTP clients: reqwest, surf)
- **opentelemetry-stackdriver**: Google Cloud Stackdriver/Cloud Trace exporter
- **opentelemetry-exporter-geneva/**: Microsoft Geneva telemetry exporter (3 sub-crates)
  - `geneva-uploader`: Core uploader for Geneva backend
  - `geneva-uploader-ffi`: FFI layer for cross-language integration
  - `opentelemetry-exporter-geneva`: OpenTelemetry-compliant Geneva exporter

### Platform-Specific Integrations
- **opentelemetry-etw-logs**: Event Tracing for Windows (ETW) logs (Windows-only)
- **opentelemetry-etw-metrics**: Event Tracing for Windows metrics (Windows-only)
- **opentelemetry-user-events-logs**: Linux user_events logs (Linux-only)
- **opentelemetry-user-events-metrics**: Linux user_events metrics (Linux-only)
- **opentelemetry-user-events-trace**: Linux user_events tracing (Linux-only)

### Instrumentation Libraries
- **opentelemetry-instrumentation-actix-web**: Actix-web framework instrumentation
- **opentelemetry-instrumentation-tower**: Tower middleware for HTTP metrics

### Utilities
- **opentelemetry-contrib**: Community propagators and exporters (base64, binary propagator, Jaeger JSON)
- **opentelemetry-resource-detectors**: Auto-detection of resource attributes from environment

## Architecture Notes

### Workspace Dependencies
Common OpenTelemetry dependencies are managed at the workspace level in the root `Cargo.toml`:
- `opentelemetry = "0.31"`
- `opentelemetry_sdk = "0.31"`
- `opentelemetry-proto = "0.31"`

Individual crates can reference these with `workspace = true`.

### Feature-Based Design
Most crates use feature flags extensively to support different:
- HTTP clients (reqwest-blocking-client, reqwest-client, surf-client)
- Async runtimes (rt-tokio, rt-tokio-current-thread)
- Optional functionality (tls-native-roots, tls-webpki-roots, gcp-authorizer)

When testing or building, use `--all-features` for comprehensive coverage, or specify features explicitly for targeted testing.

### Platform-Specific Code
ETW crates (Windows) and user-events crates (Linux) contain platform-specific code. CI runs clippy on all platforms (Ubuntu, Windows, macOS, ARM) to catch platform-specific issues.

### Error Handling
- All trace errors must be wrapped in `opentelemetry::trace::TraceError`
- All metrics errors must be wrapped in `opentelemetry::metrics::MetricsError`
- Custom exporters should implement the `ExporterError` trait

### Configuration Priority
1. Environment variables (highest priority)
2. Compile-time configurations in source code

## Contributing Guidelines

### Commit Message Format
Follow [conventional commits](https://www.conventionalcommits.org/en/v1.0.0/) standard (e.g., `feat:`, `fix:`, `chore:`).

### Code Style
- Follow Rust idioms over spec API structure compliance
- Prefer language-native patterns even if they differ from OpenTelemetry specification structure
- Configuration should prioritize environment variables over code

### CI Requirements
All PRs must pass:
- Format check: `cargo fmt --all -- --check`
- Clippy on all platforms (Ubuntu, Windows, macOS, ARM)
- All tests with all features
- Documentation builds without warnings
- MSRV compatibility check

### Running Single Tests
```bash
# Run specific test
cargo test <test_name> -p <crate-name>

# Run tests with specific features
cargo test -p <crate-name> --features "<feature1>,<feature2>"
```

## Special Notes

### Protobuf Dependency
Some crates require protoc (Protocol Buffers compiler). CI uses `arduino/setup-protoc` action. Locally, install via:
```bash
# macOS
brew install protobuf

# Ubuntu
apt-get install protobuf-compiler
```

### Geneva Exporter
The Geneva exporter is Microsoft-internal infrastructure and not intended for external use. It's open-sourced for transparency.

### Examples Directory
The `examples/` directory and crate-specific example subdirectories contain working examples. These may have pinned dependency versions for stability.
