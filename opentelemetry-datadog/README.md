# OpenTelemetry Datadog

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

> **⚠️ DEPRECATED** — This crate is deprecated and will be removed from this
> repository. Datadog now ships [`dd-trace-rs`], a first-party integration
> built on top of `opentelemetry_sdk`. New users should adopt `dd-trace-rs`,
> and existing users are encouraged to migrate. The last release of this crate
> on crates.io will remain available, but no further releases are planned.
>
> See [issue #609] for context.
>
> [`dd-trace-rs`]: https://github.com/DataDog/dd-trace-rs
> [issue #609]: https://github.com/open-telemetry/opentelemetry-rust-contrib/issues/609

| Status        |                                            |
| ------------- |--------------------------------------------|
| Stability     | deprecated                                 |
| Owners        | _unmaintained_                             |

Community supported vendor integrations for applications instrumented with [`OpenTelemetry`].

[![Crates.io: opentelemetry-datadog](https://img.shields.io/crates/v/opentelemetry-datadog.svg)](https://crates.io/crates/opentelemetry-datadog)
[![Documentation](https://docs.rs/opentelemetry-datadog/badge.svg)](https://docs.rs/opentelemetry-datadog)
[![Slack](https://img.shields.io/badge/slack-@cncf/otel/rust-brightgreen.svg?logo=slack)](https://cloud-native.slack.com/archives/C03GDP0H023)

## Overview

[`OpenTelemetry`] is a collection of tools, APIs, and SDKs used to instrument,
generate, collect, and export telemetry data (metrics, logs, and traces) for
analysis in order to understand your software's performance and behavior. This
crate provides additional propagators and exporters for sending telemetry data
to [`Datadog`].

## Features

`opentelemetry-datadog` supports following features:

- `agent-sampling`: move decision making about sampling to `datadog-agent` (see `agent_sampling.rs` example).
- `reqwest-blocking-client`: use `reqwest` blocking http client to send spans.
- `reqwest-client`: use `reqwest` http client to send spans. May not work with BatchProcessor.
- `surf-client`: use `surf` http client to send spans.


## Kitchen Sink Full Configuration

 [Example](https://docs.rs/opentelemetry-datadog/latest/opentelemetry_datadog/#kitchen-sink-full-configuration) showing how to override all configuration options. See the
 [`DatadogPipelineBuilder`] docs for details of each option.

 [`DatadogPipelineBuilder`]: https://docs.rs/opentelemetry-datadog/latest/opentelemetry_datadog/struct.DatadogPipelineBuilder.html

[`Datadog`]: https://www.datadoghq.com/
[`OpenTelemetry`]: https://crates.io/crates/opentelemetry
