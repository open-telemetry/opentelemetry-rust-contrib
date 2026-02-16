# OpenTelemetry AWS

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

| Status        |           |
| ------------- |-----------|
| Stability     | beta      |
| Owners        | [Jonathan Lee](https://github.com/jj22ee), [Jérémie Rodon](https://github.com/JeremieRodon) |

Additional types for exporting [`OpenTelemetry`] data to AWS.

[![Crates.io: opentelemetry-aws](https://img.shields.io/crates/v/opentelemetry-aws.svg)](https://crates.io/crates/opentelemetry-aws)
[![Documentation](https://docs.rs/opentelemetry-aws/badge.svg)](https://docs.rs/opentelemetry-aws)
[![LICENSE](https://img.shields.io/crates/l/opentelemetry-aws)](./LICENSE)
[![GitHub Actions CI](https://github.com/open-telemetry/opentelemetry-rust-contrib/workflows/CI/badge.svg)](https://github.com/open-telemetry/opentelemetry-rust-contrib/actions?query=workflow%3ACI+branch%3Amain)
[![Slack](https://img.shields.io/badge/slack-@cncf/otel/rust-brightgreen.svg?logo=slack)](https://cloud-native.slack.com/archives/C03GDP0H023)

## Overview

[`OpenTelemetry`] is a collection of tools, APIs, and SDKs used to instrument,
generate, collect, and export telemetry data (metrics, logs, and traces) for
analysis in order to understand your software's performance and behavior. This
crate provides additional propagators and exporters for sending telemetry data
to AWS's telemetry platform.

## Components

| Component | Feature | Description |
|-----------|---------|-------------|
| **X-Ray Propagator** | `trace` | Propagates trace context using the `X-Amzn-Trace-Id` header |
| **X-Ray ID Generator** | `trace` | Generates X-Ray-compatible trace and span IDs (time-based trace IDs) |
| **X-Ray Exporter** | `xray-exporter` | Exports OpenTelemetry spans as [X-Ray segment documents] |
| **Lambda Resource Detector** | `detector-aws-lambda` | Detects AWS Lambda resource attributes from the environment |

## Quick Start

Basic setup forwarding Segments to the local X-Ray daemon (requires `xray-daemon-client` feature):

```rust,no_run
use opentelemetry_aws::{
    trace::XrayIdGenerator,
    xray_exporter::{XrayExporter, daemon_client::XrayDaemonClient},
};
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry::global;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a client that sends to the X-Ray daemon on localhost:2000 (UDP)
    let client = XrayDaemonClient::default();

    // Create the exporter
    let exporter = XrayExporter::new(client);

    // Build and register the tracer provider
    let provider = SdkTracerProvider::builder()
        .with_id_generator(XrayIdGenerator::default())
        .with_batch_exporter(exporter)
        .build();

    global::set_tracer_provider(provider);
    Ok(())
}
```

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `trace` | Yes | X-Ray propagator and ID generator |
| `xray-exporter` | Yes | X-Ray span exporter with `SegmentTranslator` and `SegmentDocumentExporter` trait |
| `xray-daemon-client` | No | `XrayDaemonClient` — UDP client for the [X-Ray daemon] |
| `xray-stdout-client` | No | `StdoutClient` — writes segment documents to stdout (useful for debugging) |
| `subsegment-nesting` | No | Enables subsegment nesting within parent segments during translation |
| `detector-aws-lambda` | No | AWS Lambda resource detector |
| `internal-logs` | Yes | Internal instrumentation via the `tracing` crate |

[`OpenTelemetry`]: https://crates.io/crates/opentelemetry
[X-Ray daemon]: https://docs.aws.amazon.com/xray/latest/devguide/xray-daemon.html
[X-Ray segment documents]: https://docs.aws.amazon.com/xray/latest/devguide/xray-api-segmentdocuments.html
