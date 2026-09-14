# OpenTelemetry Stackdriver — removed

`opentelemetry-stackdriver` has been removed from this repository after its deprecation in version 0.29.1, released on August 5, 2026. No further releases are planned.

Migrate to OTLP using [`opentelemetry-otlp`](https://crates.io/crates/opentelemetry-otlp). Google Cloud supports OTLP ingestion directly, and the OpenTelemetry Collector also provides a [`googlecloud` exporter](https://github.com/open-telemetry/opentelemetry-collector-contrib/tree/main/exporter/googlecloudexporter).

Existing [crates.io releases](https://crates.io/crates/opentelemetry-stackdriver) remain available. The final release's [source and changelog](https://github.com/open-telemetry/opentelemetry-rust-contrib/tree/opentelemetry-stackdriver-0.29.1/opentelemetry-stackdriver) are preserved in its release tag.

See [#609](https://github.com/open-telemetry/opentelemetry-rust-contrib/issues/609) for the deprecation discussion and [#674](https://github.com/open-telemetry/opentelemetry-rust-contrib/issues/674) for the removal plan.
