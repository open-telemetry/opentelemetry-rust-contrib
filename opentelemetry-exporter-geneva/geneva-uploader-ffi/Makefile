# Makefile for Geneva FFI testing

.PHONY: all build test clean c-example rust-tests

# Default target
all: build test

# Build the Rust FFI library
build:
	cargo build --release

# Run Rust unit tests
rust-tests:
	cargo test


# Build and run C example
c-example: build
	@echo "Building C example..."
	gcc -o examples/c_example_test \
		examples/c_example.c \
		-I./include \
		-L../../target/release \
		-lgeneva_uploader_ffi \
		-lpthread -ldl -lm
	@echo "Running C example..."
	LD_LIBRARY_PATH=../../target/release ./examples/c_example_test

# Run all tests
test: rust-tests c-example

# Clean build artifacts
clean:
	cargo clean
	rm -f examples/c_example_test

# Check that the FFI library builds correctly
check:
	cargo check

# Build with debug symbols for easier debugging
debug:
	cargo build

# Generate documentation
docs:
	cargo doc --open

# Verify C header matches Rust definitions
verify-header:
	@echo "Verifying C header compatibility..."
	gcc -c examples/c_example.c -I./include -o /tmp/test_header.o && echo "✓ C header is valid" || echo "✗ C header has issues"
	rm -f /tmp/test_header.o

help:
	@echo "Available targets:"
	@echo "  all                  - Build and test everything"
	@echo "  build               - Build Rust FFI library"
	@echo "  rust-tests          - Run Rust unit tests"
	@echo "  c-example           - Build and run C example"
	@echo "  test                - Run all tests"
	@echo "  check               - Check that code compiles"
	@echo "  debug               - Build with debug symbols"
	@echo "  clean               - Clean build artifacts"
	@echo "  docs                - Generate documentation"
	@echo "  verify-header       - Verify C header compatibility"
	@echo "  help                - Show this help"
