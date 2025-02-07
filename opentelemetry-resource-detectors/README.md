# OpenTelemetry Resource Detectors

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

| Status        |                                            |
| ------------- |--------------------------------------------|
| Stability     | beta                                       |
| Owners        | [Anton Grübel](https://github.com/gruebel) |

Community supported Resource detectors implementations for applications instrumented with [`OpenTelemetry`].

[![Crates.io: opentelemetry-resource-detectors](https://img.shields.io/crates/v/opentelemetry-resource-detectors.svg)](https://crates.io/crates/opentelemetry-resource-detectors)
[![Documentation](https://docs.rs/opentelemetry-resource-detectors/badge.svg)](https://docs.rs/opentelemetry-resource-detectors)
[![LICENSE](https://img.shields.io/crates/l/opentelemetry-resource-detectors)](./LICENSE)
[![GitHub Actions CI](https://github.com/open-telemetry/opentelemetry-rust-contrib/workflows/CI/badge.svg)](https://github.com/open-telemetry/opentelemetry-rust-contrib/actions?query=workflow%3ACI+branch%3Amain)
[![Slack](https://img.shields.io/badge/slack-@cncf/otel/rust-brightgreen.svg?logo=slack)](https://cloud-native.slack.com/archives/C03GDP0H023)

## Overview

[`The opentelemetry-resource-detectors`] crate provides a collection of tools to detect resources from the runtime. This crate provides additional detectors for OS and process-related resources. Despite not being included in the SDK due to their lack of stability, these detectors can facilitate gathering additional telemetry information.

## Features

| Detector                | Implemented Resources                               | OS Supported | Semantic Conventions                                                                      |
|-------------------------| --------------------------------------------------- |--------------|-------------------------------------------------------------------------------------------|
| ProcessResourceDetector | PROCESS_COMMAND_ARGS, PROCESS_PID | all          | https://github.com/open-telemetry/semantic-conventions/blob/main/docs/resource/process.md |
| OsResourceDetector      | OS_TYPE | all          | https://github.com/open-telemetry/semantic-conventions/blob/main/docs/resource/os.md      |
| HostResourceDetector    | HOST_ID | linux        | https://github.com/open-telemetry/semantic-conventions/blob/main/docs/resource/host.md    |
| HostResourceDetector    | HOST_ARCH | all        | https://github.com/open-telemetry/semantic-conventions/blob/main/docs/resource/host.md    |
