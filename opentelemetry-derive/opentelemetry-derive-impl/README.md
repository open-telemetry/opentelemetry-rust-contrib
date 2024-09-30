![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

# OpenTelemetry derive macro implementations

Implementation of derive macros for [`OpenTelemetry`].

[![Crates.io: opentelemetry-derive-impl](https://img.shields.io/crates/v/opentelemetry-derive-impl.svg)](https://crates.io/crates/opentelemetry-derive-impl)
[![Documentation](https://docs.rs/opentelemetry-derive-impl/badge.svg)](https://docs.rs/opentelemetry-derive-impl)
[![LICENSE](https://img.shields.io/crates/l/opentelemetry-derive-impl)](./LICENSE)
[![GitHub Actions CI](https://github.com/open-telemetry/opentelemetry-rust-contrib/workflows/CI/badge.svg)](https://github.com/open-telemetry/opentelemetry-rust-contrib/actions?query=workflow%3ACI+branch%3Amain)
[![Slack](https://img.shields.io/badge/slack-@cncf/otel/rust-brightgreen.svg?logo=slack)](https://cloud-native.slack.com/archives/C03GDP0H023)

## Overview

[`OpenTelemetry`] is a collection of tools, APIs, and SDKs used to instrument,
generate, collect, and export telemetry data (metrics, logs, and traces) for
analysis in order to understand your software's performance and behavior.

This crate provides the implementation of derive macros, it should not be used directly,
but through the `opentelemetry-derive` crate.

[`OpenTelemetry`]: https://crates.io/crates/opentelemetry
