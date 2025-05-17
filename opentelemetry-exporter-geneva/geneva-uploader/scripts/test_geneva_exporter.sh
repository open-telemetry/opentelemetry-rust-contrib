#!/bin/bash

set -eu

echo "Running tests for opentelemetry-exporter-geneva with --all-features"
cargo test -p opentelemetry-exporter-geneva --all-features --tests

echo "Running doctests for opentelemetry-exporter-geneva with --all-features"
cargo test -p opentelemetry-exporter-geneva --all-features --doc