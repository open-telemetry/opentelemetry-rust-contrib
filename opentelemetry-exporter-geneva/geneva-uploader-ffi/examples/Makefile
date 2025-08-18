# Makefile for building and running the C FFI example (local to examples/)
# This focuses only on compiling c_example.c against the built Rust FFI libs.

.PHONY: all run build-rust build-otlp c-example clean verify-header help

# Defaults: build only (do not run)
all: build-rust build-otlp $(BINARY)

# Paths relative to this examples/ directory
RUST_CRATE_DIR := ..
INCLUDE_DIR    := ../include
LIB_DIR        := ../../../target/release
LIB_DIR_DEPS   := $(LIB_DIR)/deps
BINARY         := c_example_test

# Build the Rust FFI library (release)
build-rust:
	@echo "Building Rust FFI library..."
	@cd $(RUST_CRATE_DIR) && cargo build --release

# Build the example-only otlp_builder (release)
build-otlp:
	@echo "Building otlp_builder (cdylib + rlib)..."
	@cd otlp_builder && cargo build --release

# Build the C example binary
$(BINARY): c_example.c build-rust build-otlp
	@echo "Building C example..."
	@gcc -std=c11 -o $(BINARY) c_example.c \
		-I$(INCLUDE_DIR) \
		-L$(LIB_DIR) -L$(LIB_DIR_DEPS) \
		-lgeneva_uploader_ffi -lotlp_builder \
		-Wl,-rpath,@loader_path/../../../target/release \
		-Wl,-rpath,@loader_path/../../../target/release/deps \
		-lpthread -ldl -lm

# Run the example with proper dynamic library path for macOS/Linux
run: $(BINARY)
	@echo "Running C example..."
	@DYLD_LIBRARY_PATH=$(LIB_DIR):$(LIB_DIR_DEPS) LD_LIBRARY_PATH=$(LIB_DIR):$(LIB_DIR_DEPS) ./$(BINARY)

# Alias
c-example: run

# Quick header verification (compile-only)
verify-header:
	@echo "Verifying C header compatibility..."
	@gcc -c c_example.c -I$(INCLUDE_DIR) -o /tmp/test_header.o && echo "✓ C header is valid" || echo "✗ C header has issues"
	@rm -f /tmp/test_header.o

# Clean example build artifacts
clean:
	@rm -f $(BINARY)

help:
	@echo "Targets:"
	@echo "  all           - Build Rust lib + otlp_builder + C example binary (default)"
	@echo "  run           - Build and run the C example (requires env vars)"
	@echo "  c-example     - Same as 'run'"
	@echo "  build-rust    - Build the Rust FFI library in release mode"
	@echo "  build-otlp    - Build the otlp_builder cdylib/rlib (release)"
	@echo "  verify-header - Compile-check the header with the example"
	@echo "  clean         - Remove the C example binary"
