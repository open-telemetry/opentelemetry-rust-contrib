# geneva-uploader

The geneva-uploader is designed for Microsoft products to send data to public-facing end-points which route to Microsoft's internal data pipeline. It is not meant to be used outside of Microsoft products and is open sourced to demonstrate best practices and to be transparent about what is being collected.

Core uploader responsible for encoding and sending telemetry data to the Geneva backend.

## Ingestion paths

There are two ways to send logs:

### 1. OpenTelemetry SDK path (standard)

Use the `opentelemetry-exporter-geneva` crate which wires `GenevaClient` into
the OTel SDK pipeline (`SdkLoggerProvider` → `GenevaExporter`).  This is the
right choice when your application already uses the OTel SDK or the
`tracing` crate.

### 2. Log view path (direct / zero-copy)

Use `GenevaClient::encode_and_compress_logs_view` when your telemetry data is
already held in a type that implements
[`LogsDataView`](https://docs.rs/otap-df-pdata-views) — for example an
Arrow-backed view from otap-dataflow.  This path bypasses the OTel SDK
entirely and avoids copying data into proto structs.

```rust
// Implement LogsDataView for your data type, then:
let batches = client.encode_and_compress_logs_view(&my_view)?;
for batch in &batches {
    client.upload_batch(batch).await?;
}
```

The `LogRecordView::event_name` value controls which Geneva event table each
record is routed to.  Records with no event name default to the `"Log"` table.

See `examples/view_basic.rs` for a complete working example including a
minimal `LogsDataView` implementation.
