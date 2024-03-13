#!/bin/bash

set -eu

cargo_feature() {
    echo "checking $1 with features $2"
    cargo clippy --manifest-path=$1/Cargo.toml --all-targets --features "$2" --no-default-features -- \
    `# Exit with a nonzero code if there are clippy warnings` \
    -Dwarnings
}

if rustup component add clippy; then
  cargo clippy --all-targets --all-features -- \
    `# Exit with a nonzero code if there are clippy warnings` \
    -Dwarnings

  cargo_feature opentelemetry-aws "default"

# TODO: Can re-enable once back in the workspace.
#  cargo_feature opentelemetry-datadog "reqwest-blocking-client"
#  cargo_feature opentelemetry-datadog "reqwest-client"
#  cargo_feature opentelemetry-datadog "surf-client"

  cargo_feature opentelemetry-contrib "default"
  cargo_feature opentelemetry-contrib "api"
  cargo_feature opentelemetry-contrib "base64_format"
  cargo_feature opentelemetry-contrib "binary_propagator"
  cargo_feature opentelemetry-contrib "jaeger_json_exporter"
  cargo_feature opentelemetry-contrib "rt-tokio"
  cargo_feature opentelemetry-contrib "rt-tokio-current-thread"
  cargo_feature opentelemetry-contrib "rt-async-std"

# TODO: Can re-enable once back in the workspace.
#  cargo_feature opentelemetry-stackdriver "default"
#  cargo_feature opentelemetry-stackdriver "yup-authorizer"
#  cargo_feature opentelemetry-stackdriver "tls-native-roots"
#  cargo_feature opentelemetry-stackdriver "tls-webpki-roots"

  cargo_feature opentelemetry-user-events-logs "default"
  cargo_feature opentelemetry-user-events-logs "logs_level_enabled"

  cargo_feature opentelemetry-user-events-metrics ""

  cargo_feature opentelemetry-resource-detector ""
fi
