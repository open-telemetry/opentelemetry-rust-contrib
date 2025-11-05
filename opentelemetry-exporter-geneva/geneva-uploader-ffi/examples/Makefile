# Makefile for building and running the C FFI example (local to examples/)
# This focuses only on compiling c_example.c against the built Rust FFI libs.

.PHONY: all run build-rust build-otlp logs-example spans-example clean verify-header help

# Defaults: build only (do not run)
all: build-rust build-otlp $(LOGS_BINARY) $(SPANS_BINARY)

# Paths relative to this examples/ directory
RUST_CRATE_DIR := ..
INCLUDE_DIR    := ../include
LIB_DIR        := ../../../target/release
LIB_DIR_DEPS   := $(LIB_DIR)/deps
LOGS_BINARY    := logs_example_test
SPANS_BINARY   := spans_example_test

# Build the Rust FFI library (release)
build-rust:
	@echo "Building Rust FFI library..."
	@cd $(RUST_CRATE_DIR) && cargo build --release

# Build the example-only otlp_builder (release)
build-otlp:
	@echo "Building otlp_builder (cdylib + rlib)..."
	@cd otlp_builder && cargo build --release

# Build the logs example binary
$(LOGS_BINARY): logs_example.c build-rust build-otlp
	@echo "Building logs example..."
	@gcc -std=c11 -o $(LOGS_BINARY) logs_example.c \
		-I$(INCLUDE_DIR) \
		-L$(LIB_DIR) -L$(LIB_DIR_DEPS) \
		-lgeneva_uploader_ffi -lotlp_builder \
		-Wl,-rpath,@loader_path/../../../target/release \
		-Wl,-rpath,@loader_path/../../../target/release/deps \
		-lpthread -ldl -lm

# Build the spans example binary
$(SPANS_BINARY): spans_example.c build-rust build-otlp
	@echo "Building spans example..."
	@gcc -std=c11 -o $(SPANS_BINARY) spans_example.c \
		-I$(INCLUDE_DIR) \
		-L$(LIB_DIR) -L$(LIB_DIR_DEPS) \
		-lgeneva_uploader_ffi -lotlp_builder \
		-Wl,-rpath,@loader_path/../../../target/release \
		-Wl,-rpath,@loader_path/../../../target/release/deps \
		-lpthread -ldl -lm

# Run the logs example with proper dynamic library path for macOS/Linux
run-logs: $(LOGS_BINARY)
	@echo "Running logs example..."
	@DYLD_LIBRARY_PATH=$(LIB_DIR):$(LIB_DIR_DEPS) LD_LIBRARY_PATH=$(LIB_DIR):$(LIB_DIR_DEPS) ./$(LOGS_BINARY)

# Run the spans example with proper dynamic library path for macOS/Linux
run-spans: $(SPANS_BINARY)
	@echo "Running spans example..."
	@DYLD_LIBRARY_PATH=$(LIB_DIR):$(LIB_DIR_DEPS) LD_LIBRARY_PATH=$(LIB_DIR):$(LIB_DIR_DEPS) ./$(SPANS_BINARY)

# Aliases
logs-example: run-logs
spans-example: run-spans

# Quick header verification (compile-only)
verify-header:
	@echo "Verifying C header compatibility..."
	@gcc -c logs_example.c -I$(INCLUDE_DIR) -o /tmp/test_logs_header.o && echo "✓ Logs example C header is valid" || echo "✗ Logs example C header has issues"
	@gcc -c spans_example.c -I$(INCLUDE_DIR) -o /tmp/test_spans_header.o && echo "✓ Spans example C header is valid" || echo "✗ Spans example C header has issues"
	@rm -f /tmp/test_logs_header.o /tmp/test_spans_header.o

# Clean example build artifacts
clean:
	@rm -f $(LOGS_BINARY) $(SPANS_BINARY)

help:
	@echo "Targets:"
	@echo "  all           - Build Rust lib + otlp_builder + both example binaries (default)"
	@echo "  run-logs      - Build and run the logs example (requires env vars)"
	@echo "  run-spans     - Build and run the spans example (requires env vars)"
	@echo "  logs-example  - Same as 'run-logs'"
	@echo "  spans-example - Same as 'run-spans'"
	@echo "  build-rust    - Build the Rust FFI library in release mode"
	@echo "  build-otlp    - Build the otlp_builder cdylib/rlib (release)"
	@echo "  verify-header - Compile-check the headers with both examples"
	@echo "  clean         - Remove both example binaries"
