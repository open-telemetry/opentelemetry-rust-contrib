# Makefile for the split-artifact c-basic-traces example.
#
# Builds BOTH Rust libraries (opentelemetry-c-api and opentelemetry-c-sdk) and links this
# C program against both. Demonstrates the supported linking model: an application links
# the SDK plus the API; instrumentation (the do_instrumentation_work() calls) uses only the
# API surface.
#
#   make            # build both Rust libs + the example binary
#   make run        # build, then run (exports to localhost:4318 by default)
#   make clean      # remove the example binary

.PHONY: all run clean

API_INC := ../../../api/include
SDK_INC := ../../include
LIB_DIR := ../../../../target/release
BINARY  := c-basic-traces

CC    ?= cc
CSTD  ?= -std=c11
CFLAGS ?= -Wall -Wextra -O2

UNAME_S := $(shell uname -s)
ifeq ($(UNAME_S),Darwin)
    RPATH    := -Wl,-rpath,@loader_path/$(LIB_DIR)
else
    RPATH    := -Wl,-rpath,$$ORIGIN/$(LIB_DIR)
endif

all: $(BINARY)

build-rust:
	@echo "Building opentelemetry-c-api and opentelemetry-c-sdk (release)..."
	@cd ../../../.. && cargo build --release -p opentelemetry-c-api -p opentelemetry-c-sdk

$(BINARY): main.c build-rust
	@echo "Linking $(BINARY) against libopentelemetry_c_api + libopentelemetry_c_sdk..."
	@$(CC) $(CSTD) $(CFLAGS) -o $(BINARY) main.c \
		-I$(API_INC) -I$(SDK_INC) \
		-L$(LIB_DIR) -lopentelemetry_c_api -lopentelemetry_c_sdk \
		$(RPATH)
	@echo "Built ./$(BINARY)"

run: $(BINARY)
	@DYLD_LIBRARY_PATH=$(LIB_DIR) LD_LIBRARY_PATH=$(LIB_DIR) ./$(BINARY)

clean:
	@rm -f $(BINARY)
