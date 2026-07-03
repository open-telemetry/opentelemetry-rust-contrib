#!/bin/bash

set -eu

# The cross-artifact proof test (opentelemetry-c-sdk `cross_artifact`) links a C program
# against the built opentelemetry-c-api and opentelemetry-c-sdk *cdylibs*, installs the SDK,
# and asserts API-only spans export through it. `cargo test` does not emit cdylib artifacts,
# so build them explicitly first, with the SAME feature set the tests use (`--all-features`)
# so the validated artifact matches the test command. Without this prebuild the test would
# self-skip locally, or fail under CI (where CI=true makes it fail-hard rather than skip, so
# the shared-global proof cannot silently no-op).
#
# This runs on every Unix CI runner (Linux and macOS), so it enforces the supported
# Unix-like dynamic-linking artifact model — including macOS, whose cdylib link relies on
# `opentelemetry-c/sdk/build.rs` emitting `-undefined dynamic_lookup`.
echo "Building opentelemetry-c-api / opentelemetry-c-sdk cdylibs for the cross-artifact test"
cargo build -p opentelemetry-c-api -p opentelemetry-c-sdk --all-features

echo "Running tests for all packages in workspace with --all-features"
cargo test --workspace --all-features --tests

echo "Running doctests for all packages in workspace with --all-features"
cargo test --workspace --all-features --doc
