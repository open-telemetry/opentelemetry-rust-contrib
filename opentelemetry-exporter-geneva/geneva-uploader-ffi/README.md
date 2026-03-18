# geneva-uploader-ffi

The geneva-uploader-ffi is designed for Microsoft products to send data to public-facing end-points which route to Microsoft's internal data pipeline. It is not meant to be used outside of Microsoft products and is open sourced to demonstrate best practices and to be transparent about what is being collected.

C/C++ FFI layer over `geneva-uploader`. Exposes a stable C ABI so that C++ agents can encode and upload telemetry to Geneva without pulling in the Rust OTel SDK.

## Ingestion paths

### 1. OTLP bytes path

Pass a serialised `ExportLogsServiceRequest` (or `ExportTraceServiceRequest`) protobuf. Rust deserialises it and encodes to Geneva Bond+LZ4.

```c
GenevaError rc = geneva_encode_and_compress_logs(
    client, proto_bytes, proto_len, &batches, err, sizeof(err));
```

Use this when your records already carry resource or instrumentation-scope attributes (service name, host, etc.) that need to be preserved.

### 2. Direct log record path (zero-copy)

Pass a flat C array of `GenevaLogRecordC` structs. Rust reads the fields directly from C memory — no intermediate OTLP serialisation, no copies of string data.

```c
GenevaLogRecordC records[] = {
    {
        .event_name      = "Log",
        .time_unix_nano  = now_nanos(),
        .severity_number = 9,       /* INFO */
        .body            = "hello from C++",
        .attr_keys       = keys,
        .attr_values     = vals,
        .attr_count      = 2,
    },
};
GenevaError rc = geneva_encode_and_compress_log_records(
    client, records, 1, &batches, err, sizeof(err));
```

All pointers inside `GenevaLogRecordC` only need to stay valid for the duration of the encode call. After it returns the caller may free or reuse every buffer.

Use this when you have flat log records without resource/scope metadata and want the lowest possible overhead.

See `examples/log_records_example.c` for a complete working example.

## Call sequence (both paths)

```
geneva_client_new()
  └─ geneva_encode_and_compress_logs()        ← OTLP path
     or
     geneva_encode_and_compress_log_records() ← direct path
        └─ geneva_batches_len()
        └─ geneva_upload_batch_sync() × N
        └─ geneva_batches_free()
geneva_client_free()
```

## C header

All types and function signatures are declared in `include/geneva_ffi.h`.
Error codes are in `include/geneva_errors.h`.
