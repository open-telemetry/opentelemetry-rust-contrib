#!/bin/bash

set -eu

cargo test --all --all-features "$@" -- --test-threads=1


cargo test --manifest-path=opentelemetry-aws/Cargo.toml --all-features
cargo test --manifest-path=opentelemetry-contrib/Cargo.toml --all-features

cargo test --manifest-path=opentelemetry-datadog/Cargo.toml --all-features
cargo test --manifest-path=opentelemetry-stackdriver/Cargo.toml --all-features

cargo test --manifest-path=opentelemetry-user-events-logs/Cargo.toml --all-features
cargo test --manifest-path=opentelemetry-user-events-metrics/Cargo.toml --all-features

cargo test --manifest-path=opentelemetry-resource-detectors/Cargo.toml --all-features
