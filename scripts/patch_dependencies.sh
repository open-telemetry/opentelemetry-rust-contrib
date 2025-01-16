#!/bin/bash

function patch_version() {
  local latest_version=$(cargo search --limit 1 $1 | head -1 | cut -d'"' -f2)
  echo "patching $1 from $latest_version to $2"
  cargo update -p $1:$latest_version --precise $2
}

patch_version home 0.5.5 # for opentelemetry-stackdriver
patch_version url 2.4.1 # for opentelemetry-datadog
