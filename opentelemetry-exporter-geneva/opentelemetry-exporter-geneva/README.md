# opentelemetry-exporter-geneva

| Status        |           |
| ------------- |-----------|
| Stability     | alpha     |
| Owners        | [Lalit Kumar Bhasin](https://github.com/lalitb)       |

The opentelemetry-exporter-geneva is designed for Microsoft products to send data to public-facing end-points which route to Microsoft's internal data pipeline. It is not meant to be used outside of Microsoft products and is open sourced to demonstrate best practices and to be transparent about what is being collected.

OpenTelemetry-compliant exporter for Geneva, providing both log and trace exporters
that integrate with the OpenTelemetry SDK pipeline.

## Exporters

- **`GenevaExporter`** — implements `opentelemetry_sdk::logs::LogExporter` for sending logs
  to Geneva via the OTel SDK `SdkLoggerProvider`.
- **`GenevaTraceExporter`** — implements `opentelemetry_sdk::trace::SpanExporter` for sending
  traces to Geneva via the OTel SDK `TracerProvider`.

## Usage

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
opentelemetry-exporter-geneva = "0.5.0"
```

See `examples/basic.rs` for a logs example and `examples/trace_basic.rs` for
a traces example.
