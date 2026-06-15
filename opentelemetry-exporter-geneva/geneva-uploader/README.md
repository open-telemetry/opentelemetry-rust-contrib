# geneva-uploader

The geneva-uploader is designed for Microsoft products to send data to public-facing end-points which route to Microsoft's internal data pipeline. It is not meant to be used outside of Microsoft products and is open sourced to demonstrate best practices and to be transparent about what is being collected.

Core uploader responsible for encoding and sending telemetry data to the Geneva backend.

## Ingestion paths

There are two ways to send logs:

### 1. OpenTelemetry SDK path (standard)

Use the `opentelemetry-exporter-geneva` crate which wires `GenevaClient` into
the OTel SDK pipeline (`SdkLoggerProvider` → `GenevaExporter`).  This is the
right choice when your application already uses the OTel SDK or the
`tracing` crate and wants to send data directly to Geneva without using an
external Agent/Collector.

### 2. Log view path (direct / zero-copy)

Use `GenevaClient::encode_and_compress_logs` when your telemetry data is
already held in a type that implements
[`LogsDataView`](https://docs.rs/otap-df-pdata-views) — for example an
Arrow-backed view from otap-dataflow.  This path bypasses the OTel SDK
entirely and avoids copying data into proto structs.

```rust
// Implement LogsDataView for your data type, then:
let batches = client.encode_and_compress_logs(&my_view)?;
for batch in &batches {
    client.upload_batch(batch).await?;
}
```

The `LogRecordView::event_name` value controls which Geneva event table each
record is routed to.  Records with no event name default to the `"Log"` table.

See `examples/view_basic.rs` for a complete working example including a
minimal `LogsDataView` implementation.

### 3. Span upload

Use `GenevaClient::encode_and_compress_spans` when you have
`ResourceSpans` data (from the OpenTelemetry proto crate) to send
traces to Geneva.

```rust
let batches = client.encode_and_compress_spans(&resource_spans)?;
for batch in &batches {
    client.upload_batch(batch).await?;
}
```

## C/C++ FFI

For C/C++ callers, the `geneva-uploader-ffi` crate wraps `GenevaClient` behind
a stable C ABI.  It provides two encoding paths that mirror the Rust paths above:

- **OTLP bytes** (`geneva_encode_and_compress_logs`) — pass a serialised
  `ExportLogsServiceRequest` protobuf. This FFI entry point is only exported
  when `geneva-uploader-ffi` is built with the `otlp_bytes` feature.
- **Direct records** (`geneva_encode_and_compress_log_records`) — pass a flat
  `GenevaLogRecordC` array; string fields are read directly from C memory with
  no intermediate copy.

See `../geneva-uploader-ffi/README.md` for details.

## TLS backends

`geneva-uploader` supports two TLS backends, selected at compile time via Cargo features (the flags are additive; if both are enabled, `tls-rustls` is used):

- **`tls-native`** *(default)* — uses [`native-tls`] (which links against the
  system OpenSSL/SChannel/Secure Transport). This is the historical behavior
  and requires no additional setup.
- **`tls-rustls`** — uses pure-Rust [`rustls`] together with [`p12-keystore`]
  for parsing the PKCS#12 client certificate. Pick this when you need to ship
  a binary without an OpenSSL runtime dependency, or when you want to plug in a
  FIPS-validated `CryptoProvider` such as
  [`rustls-symcrypt`](https://crates.io/crates/rustls-symcrypt). Install your
  provider once at process startup:

  ```rust,ignore
  rustls_symcrypt::default_symcrypt_provider()
      .install_default()
      .expect("failed to install SymCrypt CryptoProvider");
  ```

  If no provider is installed, the rustls bundled provider (selected via
  reqwest's `rustls-tls-native-roots` feature) is used as a fallback.

To switch backends, disable defaults and select the desired feature:

```toml
[dependencies]
geneva-uploader = { version = "*", default-features = false, features = ["tls-rustls"] }
```

Both backends use the system trust store for server verification and pin the
TLS version to 1.2 (matching Geneva's required protocol). The two feature flags
are additive — if both are enabled simultaneously (e.g. via `--all-features`),
`tls-rustls` takes precedence at runtime.

[`native-tls`]: https://crates.io/crates/native-tls
[`rustls`]: https://crates.io/crates/rustls
[`p12-keystore`]: https://crates.io/crates/p12-keystore
