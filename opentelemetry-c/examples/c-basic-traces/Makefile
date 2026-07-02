# Makefile for the c-basic-traces example.
#
# Builds the opentelemetry-c Rust library (release) and links this C program against
# the resulting shared library. Tested with gcc and clang on Linux and macOS.
#
#   make            # build the Rust lib + the example binary
#   make run        # build, then run (exports to localhost:4318 by default)
#   make verify     # compile-check main.c against the headers only (no linking)
#   make clean      # remove the example binary

.PHONY: all run verify clean help

# Paths relative to this examples/c-basic-traces/ directory.
CRATE_DIR   := ../..
INCLUDE_DIR := ../../include
LIB_DIR     := ../../../target/release
BINARY      := c-basic-traces

CC    ?= cc
CSTD  ?= -std=c11
CFLAGS ?= -Wall -Wextra -O2

# Platform-specific runtime search path (rpath) and extra system libraries needed
# when linking the Rust cdylib.
UNAME_S := $(shell uname -s)
ifeq ($(UNAME_S),Darwin)
    RPATH    := -Wl,-rpath,@loader_path/$(LIB_DIR)
    SYS_LIBS :=
else
    RPATH    := -Wl,-rpath,$$ORIGIN/$(LIB_DIR)
    SYS_LIBS := -lpthread -ldl -lm
endif

all: $(BINARY)

# Build the Rust library in release mode.
build-rust:
	@echo "Building opentelemetry-c (release)..."
	@cd $(CRATE_DIR) && cargo build --release

$(BINARY): main.c build-rust
	@echo "Linking $(BINARY)..."
	@$(CC) $(CSTD) $(CFLAGS) -o $(BINARY) main.c \
		-I$(INCLUDE_DIR) \
		-L$(LIB_DIR) -lopentelemetry_c \
		$(RPATH) $(SYS_LIBS)
	@echo "Built ./$(BINARY)"

run: $(BINARY)
	@echo "Running $(BINARY)..."
	@DYLD_LIBRARY_PATH=$(LIB_DIR) LD_LIBRARY_PATH=$(LIB_DIR) ./$(BINARY)

# Compile-only check that the headers are self-contained and valid (no Rust build).
verify:
	@echo "Verifying headers compile..."
	@$(CC) $(CSTD) $(CFLAGS) -fsyntax-only -I$(INCLUDE_DIR) main.c && echo "OK: headers valid"

clean:
	@rm -f $(BINARY)

help:
	@echo "Targets:"
	@echo "  all     - build the Rust library and the example binary (default)"
	@echo "  run     - build then run the example"
	@echo "  verify  - compile-check main.c against the headers only"
	@echo "  clean   - remove the example binary"
