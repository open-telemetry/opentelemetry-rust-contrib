# OpenTelemetry Geneva Exporter

## Overview
The **OpenTelemetry Geneva Exporter** is a set of Rust-based components designed to facilitate exporting telemetry data (logs and traces) to **Geneva**. It consists of multiple crates that handle different aspects of telemetry data processing and integration.

## Project Structure
The repository contains the following sub-crates:

- **[geneva-uploader](geneva-uploader/)**: Core uploader responsible for sending telemetry data to the Geneva backend.
- **[geneva-uploader-ffi](geneva-uploader-ffi/)**: FFI (Foreign Function Interface) layer for integrating with other languages.
- **[opentelemetry-geneva-exporter](opentelemetry-geneva-exporter/)**: OpenTelemetry-compliant exporter for Geneva