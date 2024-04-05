![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

# OpenTelemetry Resource Detectors

Community supported Resource detectors implementations for applications instrumented with [`OpenTelemetry`].

## Overview

[`The opentelemetry-resource-detectors`] crate provides a collection of tools to detect resources from the runtime. This crate provides additional detectors for OS and process-related resources. Despite not being included in the SDK due to their lack of stability, these detectors can facilitate gathering additional telemetry information.

## Features

| Detector                | Implemented Resources                               | OS Supported | Semantic Conventions                                                                      |
|-------------------------| --------------------------------------------------- |--------------|-------------------------------------------------------------------------------------------|
| ProcessResourceDetector | PROCESS_COMMAND_ARGS, PROCESS_PID | all          | https://github.com/open-telemetry/semantic-conventions/blob/main/docs/resource/process.md |
| OsResourceDetector      | OS_TYPE | all          | https://github.com/open-telemetry/semantic-conventions/blob/main/docs/resource/os.md      |
| HostResourceDetector    | HOST_ID | linux        | https://github.com/open-telemetry/semantic-conventions/blob/main/docs/resource/host.md    |
| HostResourceDetector    | HOST_ARCH | all        | https://github.com/open-telemetry/semantic-conventions/blob/main/docs/resource/host.md    |
