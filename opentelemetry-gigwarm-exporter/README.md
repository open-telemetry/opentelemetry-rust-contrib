# OpenTelemetry Gigwarm Exporter

## Overview
The **OpenTelemetry Gigwarm Exporter** is a set of Rust-based components designed to facilitate exporting telemetry data (logs and traces) to **GIG/warm**. It consists of multiple crates that handle different aspects of telemetry data processing and integration.

## Project Structure
The repository contains the following sub-crates:

- **[gigwarm-uploader](gigwarm-uploader/)**: Core uploader responsible for sending telemetry data to the GIG/warm backend.
- **[gigwarm-uploader-ffi](gigwarm-uploader-ffi/)**: FFI (Foreign Function Interface) layer for integrating with other languages.
- **[opentelemetry-gigwarm-exporter](opentelemetry-gigwarm-exporter/)**: OpenTelemetry-compliant exporter for GIG/warm.