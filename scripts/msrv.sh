#!/bin/bash

set -eu

# Comma-separated package names to skip, e.g.
# MSRV_EXCLUDE_PACKAGES="geneva-uploader,geneva-uploader-ffi,opentelemetry-exporter-geneva"
exclude_packages="${MSRV_EXCLUDE_PACKAGES:-}"
# Comma-separated package names to verify only, e.g.
# MSRV_ONLY_PACKAGES="geneva-uploader,geneva-uploader-ffi,opentelemetry-exporter-geneva"
only_packages="${MSRV_ONLY_PACKAGES:-}"

members=$(cargo metadata -q --no-deps --format-version 1 | jq -r '.packages[] | [.name, .manifest_path] | @tsv')

while IFS=$'\t' read -r package_name manifest_path; do
  package_name=$(printf '%s' "$package_name" | tr -d '\r')
  manifest_path=$(printf '%s' "$manifest_path" | tr -d '\r')

  if [ -n "$only_packages" ] && ! printf ',%s,' "$only_packages" | grep -Fq ",$package_name,"; then
    echo "Skipping MSRV verification for $package_name ($manifest_path) - not in MSRV_ONLY_PACKAGES"
    echo ""
    continue
  fi

  if [ -n "$exclude_packages" ] && printf ',%s,' "$exclude_packages" | grep -Fq ",$package_name,"; then
    echo "Skipping MSRV verification for $package_name ($manifest_path)"
    echo ""
    continue
  fi

  # needed for windows CI run
  echo "Verifying MSRV version for $manifest_path"
  cargo msrv verify --manifest-path "$manifest_path" --output-format json
  echo "" # just for nicer separation between packages
done <<< "$members"
