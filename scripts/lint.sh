#!/bin/bash

set -eu

cargo_feature() {
    echo "checking $1 with features $2"
    cargo clippy --manifest-path=$1/Cargo.toml --all-targets --features "$2" --no-default-features -- \
    `# Exit with a nonzero code if there are clippy warnings` \
    -Dwarnings
}

# Exit with a nonzero code if there are clippy warnings
cargo clippy --workspace --all-targets --all-features -- -Dwarnings

cargo_feature opentelemetry-aws "default"

cargo_feature opentelemetry-c/sdk "native-tls"
cargo_feature opentelemetry-c/sdk "rustls-tls"
# OTLP exporter with no TLS backend (HTTP only).
cargo_feature opentelemetry-c/sdk "otlp"
# SDK core with OTLP compiled out entirely (no opentelemetry-otlp / reqwest / TLS).
echo "checking opentelemetry-c/sdk with no default features (SDK core)"
cargo clippy --manifest-path=opentelemetry-c/sdk/Cargo.toml --all-targets --no-default-features -- \
    -Dwarnings

cargo_feature opentelemetry-datadog "reqwest-blocking-client,intern-std"
cargo_feature opentelemetry-datadog "reqwest-client,intern-std"
# TODO: Clippy doesn't seem to like surf client.
#  cargo_feature opentelemetry-datadog "surf-client,intern-std"

cargo_feature opentelemetry-contrib "default"
cargo_feature opentelemetry-contrib "api"
cargo_feature opentelemetry-contrib "base64_format"
cargo_feature opentelemetry-contrib "binary_propagator"
cargo_feature opentelemetry-contrib "jaeger_json_exporter"
cargo_feature opentelemetry-contrib "rt-tokio"
cargo_feature opentelemetry-contrib "rt-tokio-current-thread"

cargo_feature opentelemetry-stackdriver "default"
cargo_feature opentelemetry-stackdriver "gcp-authorizer"
cargo_feature opentelemetry-stackdriver "tls-native-roots"
cargo_feature opentelemetry-stackdriver "tls-webpki-roots"

cargo_feature opentelemetry-user-events-logs "default"

cargo_feature opentelemetry-user-events-metrics ""

cargo_feature opentelemetry-resource-detectors ""
