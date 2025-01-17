# Journald Log Exporter for OpenTelemetry


![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png


This crate provides a Log Exporter to export logs to the systemd journal (journald) using the OpenTelemetry framework. The exporter supports both plain text and JSON formats for log entries and includes options for setting message size limits, identifiers, and attribute prefixes.

Journald is a system service for collecting and storing log data. It is a part of systemd, a suite of system management daemons, libraries, and utilities designed for Linux. This exporter allows OpenTelemetry to send log data directly to journald, enabling seamless integration with systemd's logging infrastructure.

This exporter requires libsystemd for sending log entries to the journald daemon.

[!CAUTION]
The exporter is in an experimental stage, and breaking changes may occur. Performance and stability improvements may be needed, and we welcome contributions.

## Features

- Export OpenTelemetry logs to journald.
- Optionally serialize logs and attributes as JSON.
- Configurable message size limit.
- Configurable attribute prefix.

## Installation and Usage

    Refer to the example under examples directory.