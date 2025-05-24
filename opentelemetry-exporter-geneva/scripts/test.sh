#!/usr/bin/env bash
set -euo pipefail

# Array of crate directories (relative to contrib root)
CRATES=(
    "opentelemetry-exporter-geneva/geneva-uploader"
    "opentelemetry-exporter-geneva/geneva-uploader-ffi"
    "opentelemetry-exporter-geneva/opentelemetry-exporter-geneva"
)

for crate in "${CRATES[@]}"; do
  echo "----------------------"
  echo "Building docs for $crate"
  echo "----------------------"
  (cd "$crate" && cargo doc --no-deps --all-features)
  echo "----------------------"
  echo "Testing $crate"
  echo "----------------------"
  (cd "$crate" && cargo test --all-features)
done
