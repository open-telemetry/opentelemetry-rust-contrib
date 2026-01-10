# OpenTelemetry Geneva Exporter

| Status        |           |
| ------------- |-----------|
| Stability     | alpha     |
| Owners        | TBD       |

The **OpenTelemetry Geneva Exporter** is designed for Microsoft products to send data to public-facing end-points which route to Microsoft's internal data pipeline. It is not meant to be used outside of Microsoft products and is open sourced to demonstrate best practices and to be transparent about what is being collected.

## Overview
The **OpenTelemetry Geneva Exporter** is a set of Rust-based components designed to facilitate exporting telemetry data (logs and traces) to **Geneva**. It consists of multiple crates that handle different aspects of telemetry data processing and integration.

## Project Structure
The repository contains the following sub-crates:

- **[geneva-uploader](geneva-uploader/)**: Core uploader responsible for sending telemetry data to the Geneva backend.
- **[geneva-uploader-ffi](geneva-uploader-ffi/)**: FFI (Foreign Function Interface) layer for integrating with other languages.
- **[opentelemetry-exporter-geneva](opentelemetry-exporter-geneva/)**: OpenTelemetry-compliant exporter for Geneva