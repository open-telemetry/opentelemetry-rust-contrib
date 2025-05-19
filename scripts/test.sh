#!/bin/bash

set -eu

echo "Running tests for all packages in workspace with --all-features"
cargo test --workspace --all-features --tests

echo "Running doctests for all packages in workspace with --all-features"
cargo test --workspace --all-features --doc
