# OpenTelemetry ZPages

> [!WARNING]  
> **This crate is deprecated and no longer maintained.**

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/master/assets/logo-text.png

ZPages server written in Rust

[![Crates.io: opentelemetry-zpages](https://img.shields.io/crates/v/opentelemetry-zpages.svg)](https://crates.io/crates/opentelemetry-zpages)
[![Documentation](https://docs.rs/opentelemetry-zpages/badge.svg)](https://docs.rs/opentelemetry-zpages)
[![LICENSE](https://img.shields.io/crates/l/opentelemetry-zpages)](./LICENSE)
[![GitHub Actions CI](https://github.com/open-telemetry/opentelemetry-rust-contrib/workflows/CI/badge.svg)](https://github.com/open-telemetry/opentelemetry-rust-contrib/actions?query=workflow%3ACI+branch%3Amain)
[![Slack](https://img.shields.io/badge/slack-@cncf/otel/rust-brightgreen.svg?logo=slack)](https://cloud-native.slack.com/archives/C03GDP0H023)

## Overview

zPages are an in-process alternative to external exporters. When included, they collect and aggregate tracing and metrics information in the background; this data is served on web pages or APIs when requested.

This crate is still working in progress. Please find its current limitations below.

Note that this crate is still in **experimental** state. Breaking changes can still happen. Some features may still in development.

## Tracez

Tracez shows information on tracing, including aggregation counts for latency, running, and errors for spans grouped by the span name.

