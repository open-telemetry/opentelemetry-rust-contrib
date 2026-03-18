# Makefile for building and running the C FFI example (local to examples/)
# This focuses only on compiling c_example.c against the built Rust FFI libs.

.PHONY: all run build-rust spans-example log-records-example clean verify-header help

# Defaults: build only (do not run)
all: build-rust $(SPANS_BINARY) $(LOG_RECORDS_BINARY)

# Paths relative to this examples/ directory
RUST_CRATE_DIR    := ..
INCLUDE_DIR       := ../include
LIB_DIR           := ../../../target/release
LIB_DIR_DEPS      := $(LIB_DIR)/deps
SPANS_BINARY      := spans_example_test
LOG_RECORDS_BINARY := log_records_example_test

# Build the Rust FFI library (release)
build-rust:
	@echo "Building Rust FFI library..."
	@cd $(RUST_CRATE_DIR) && cargo build --release

# Build the spans example binary
$(SPANS_BINARY): spans_example.c build-rust
	@echo "Building spans example..."
	@gcc -std=c11 -o $(SPANS_BINARY) spans_example.c \
		-I$(INCLUDE_DIR) \
		-L$(LIB_DIR) -L$(LIB_DIR_DEPS) \
		-lgeneva_uploader_ffi \
		-Wl,-rpath,@loader_path/../../../target/release \
		-Wl,-rpath,@loader_path/../../../target/release/deps \
		-lpthread -ldl -lm

# Build the zero-copy log records example binary (no otlp_builder needed)
$(LOG_RECORDS_BINARY): log_records_example.c build-rust
	@echo "Building log records example..."
	@gcc -std=c11 -o $(LOG_RECORDS_BINARY) log_records_example.c \
		-I$(INCLUDE_DIR) \
		-L$(LIB_DIR) \
		-lgeneva_uploader_ffi \
		-Wl,-rpath,@loader_path/../../../target/release \
		-lpthread -ldl -lm

# Run the spans example with proper dynamic library path for macOS/Linux
run-spans: $(SPANS_BINARY)
	@echo "Running spans example..."
	@DYLD_LIBRARY_PATH=$(LIB_DIR):$(LIB_DIR_DEPS) LD_LIBRARY_PATH=$(LIB_DIR):$(LIB_DIR_DEPS) ./$(SPANS_BINARY)

# Run the zero-copy log records example
run-log-records: $(LOG_RECORDS_BINARY)
	@echo "Running log records example..."
	@DYLD_LIBRARY_PATH=$(LIB_DIR) LD_LIBRARY_PATH=$(LIB_DIR) ./$(LOG_RECORDS_BINARY)

# Aliases
spans-example: run-spans
log-records-example: run-log-records

# Quick header verification (compile-only)
verify-header:
	@echo "Verifying C header compatibility..."
	@gcc -c spans_example.c -I$(INCLUDE_DIR) -o /tmp/test_spans_header.o && echo "✓ Spans example C header is valid" || echo "✗ Spans example C header has issues"
	@gcc -c log_records_example.c -I$(INCLUDE_DIR) -o /tmp/test_log_records_header.o && echo "✓ Log records example C header is valid" || echo "✗ Log records example C header has issues"
	@rm -f /tmp/test_spans_header.o /tmp/test_log_records_header.o

# Clean example build artifacts
clean:
	@rm -f $(SPANS_BINARY) $(LOG_RECORDS_BINARY)

help:
	@echo "Targets:"
	@echo "  all                - Build Rust lib + example binaries (default)"
	@echo "  run-spans          - Build and run the spans example (requires env vars)"
	@echo "  run-log-records    - Build and run the zero-copy log records example (requires env vars)"
	@echo "  spans-example      - Same as 'run-spans'"
	@echo "  log-records-example- Same as 'run-log-records'"
	@echo "  build-rust         - Build the Rust FFI library in release mode"
	@echo "  verify-header      - Compile-check the headers with all examples"
	@echo "  clean              - Remove all example binaries"
