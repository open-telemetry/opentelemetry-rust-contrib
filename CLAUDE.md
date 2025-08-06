# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository Overview

OpenTelemetry Rust Contrib provides vendor-specific exporters, platform integrations, and framework instrumentation that extend the core OpenTelemetry Rust ecosystem. The repository is organized as a Cargo workspace with components for AWS, Datadog, Google Cloud, Microsoft Geneva, Windows ETW, Linux user events, and web framework integrations.

## Essential Commands

### Primary Development Workflow
```bash
./precommit.sh              # Complete pre-commit workflow (update, fmt, lint, test)
cargo fmt --all             # Format all code
./scripts/lint.sh           # Run clippy on all feature combinations
./scripts/test.sh           # Run all tests including doctests
./scripts/msrv.sh           # Verify Minimum Supported Rust Version (1.75+)
cargo bench                  # Run benchmarks across workspace
```

### Geneva FFI-Specific Development
```bash
cd opentelemetry-exporter-geneva/geneva-uploader-ffi
make all                    # Build and test everything
make build                  # Build Rust FFI library (cdylib + staticlib)
make c-example              # Build and run C integration example
make rust-tests             # Unit tests only
make rust-integration-tests # Integration tests including FFI validation
```

### Testing Individual Components
```bash
cargo test -p <crate-name>                    # Test specific crate
cargo test --all-features                     # Test all feature combinations
cargo test --no-default-features              # Test minimal configuration
cargo test --workspace --tests                # Run only unit/integration tests (no doctests)
cargo test --workspace --doc                  # Run only documentation tests
```

### Stress Testing and Benchmarking
```bash
cd stress
cargo run --bin <stress-test-name>            # Run specific stress test
cargo bench                                   # Run benchmarks in stress directory
```

## Architecture Overview

### Workspace Organization
The repository contains three main categories of exporters:

1. **Vendor Exporters**: AWS (opentelemetry-aws), Datadog, Stackdriver, Geneva
2. **Platform Exporters**: Windows ETW, Linux user events  
3. **Framework Instrumentation**: Actix Web, Tower middleware

### Geneva Exporter Three-Tier Architecture
The Geneva exporter demonstrates sophisticated multi-language integration:

```
opentelemetry-exporter-geneva/
├── geneva-uploader/              # Core async uploader library
│   ├── config_service/           # Geneva backend configuration
│   ├── ingestion_service/        # Data upload handling
│   ├── payload_encoder/          # OTLP/Bond encoding + LZ4 compression
│   └── client.rs                 # Main client interface
├── geneva-uploader-ffi/          # C FFI layer for Go/C integration
│   ├── src/lib.rs               # FFI functions with error handling
│   ├── include/geneva_ffi.h     # C header definitions
│   └── examples/c_example.c     # Complete C integration example
└── opentelemetry-exporter-geneva/ # OpenTelemetry SDK integration
```

### Key Patterns

**Async/Tokio Integration**: All async exporters use shared Tokio runtime patterns. The FFI layer uses a single runtime per client for efficiency.

**Feature-Flag Architecture**: Extensive use of Cargo features for platform-specific functionality (`self_signed_certs`, `mock_auth`, runtime selection).

**Multi-Language FFI**: Geneva FFI provides thread-safe C interface with proper error reporting for Go CGO integration.

**Error Handling**: Structured errors using `thiserror` with thread-safe error reporting in FFI layers.

## Critical Implementation Details

### Geneva FFI Error Handling
The FFI layer uses thread-safe error storage accessible via `geneva_get_last_error()`. When implementing FFI functions, call `set_last_error()` before returning error codes to provide detailed diagnostics to calling code.

### Runtime Management
Geneva FFI creates one Tokio runtime per client handle. For applications creating multiple clients, consider runtime sharing optimizations.

### Authentication Methods
Geneva supports:
- Certificate-based (PKCS#12 files) via `AuthMethod::Certificate`  
- Azure Managed Identity via `AuthMethod::ManagedIdentity`
- Mock authentication for testing (feature-gated)

### Memory Safety in FFI
- All C string parameters must remain valid for the client's lifetime
- FFI functions validate null pointers and provide meaningful error messages
- Use `Box::into_raw()` / `Box::from_raw()` pattern for opaque handles

## Testing Strategy

### Integration Testing
- Geneva FFI includes comprehensive C integration tests
- Platform exporters have platform-specific test requirements
- Stress testing infrastructure in `/stress/` directory with dedicated benchmarks for throughput testing and exporter performance

### Feature Testing
- Test all feature flag combinations using scripts
- Platform-specific features require appropriate test environments
- Mock authentication available for Geneva testing without real credentials

## Build Configuration

### Workspace Dependencies
- OpenTelemetry core: 0.30.x series
- Tokio: 1.0+ with required features per component
- Platform-specific dependencies conditionally compiled

### Code Quality
- Rustfmt config in `rustfmt.toml` with Edition 2021 settings
- Clippy lints enforced via workspace configuration  
- `deny.toml` enforces license compatibility and security policies
- MSRV: Rust 1.75.0 minimum across all crates (verified via `scripts/msrv.sh`)

## Common Development Tasks

### Adding New Vendor Exporters
Follow the existing pattern: separate crate with OpenTelemetry SDK integration, async/sync variants as needed, comprehensive feature flags for optional dependencies.

### Platform-Specific Development
ETW (Windows) and user events (Linux) exporters provide templates for platform-specific implementations with conditional compilation.

### FFI Development
Use Geneva FFI as reference implementation for thread-safe error handling, runtime management, and C header generation patterns.