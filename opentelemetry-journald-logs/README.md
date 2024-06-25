# Journald Log Exporter for OpenTelemetry


![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png


This project provides a journald log exporter for OpenTelemetry, allowing logs to be sent to journald. Note that this exporter is experimental, and breaking changes are expected. The performance and stability improvements are required, and contributions are welcome.

## Features

- Export OpenTelemetry logs to journald.
- Optionally serialize logs and attributes as JSON.
- Configurable message size limit.
- Configurable attribute prefix.

## Installation and Usage

    Refer to the example under examples directory.