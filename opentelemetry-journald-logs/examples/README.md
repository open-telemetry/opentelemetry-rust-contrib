# Example: Basic Logs

This example demonstrates how to use the `opentelemetry-journald-logs` crate to export logs to the systemd journal (journald).

## Running the Example 

To run this example, use the following command:

```sh
cargo run --example basic-logs
```

## Running with JSON Format

To export all log properties and attributes as JSON instead of key-value pairs, uncomment the `.json_format(true)` line in the init_logger function and use the following command:

```rust
fn init_logger() -> LoggerProvider {
    let exporter = JournaldLogExporter::builder()
        .identifier("opentelemetry-journal-exporter")
        .message_size_limit(4 * 1024)
        .attribute_prefix(Some("OTEL_".to_string()))
        .json_format(true) // uncomment to log in JSON format
        .build()
        .expect("Failed to build JournaldLogExporter");

    LoggerProvider::builder()
        .with_simple_exporter(exporter)
        .build()
}
```
Then run:

```sh
cargo run --example basic-logs --features="json"
```

## Viewing the Logs

To see the logs in the systemd journal, use the following command:

```sh
journalctl -o json-seq --follow
```

### Example Output (without JSON):

```json
{
  "MESSAGE": "my test message",
  "OTEL_USER_ID": "5678",
  "_TRANSPORT": "journal",
  "_COMM": "basic-logs",
  "_UID": "1000",
  "__CURSOR": "s=b6ec4d2d7b2f458e92a4935c7915a430;i=23f19;b=92f3c733ee9a41e6a48eb22e1b60d6b5;m=154ca4caa6;t=61bb0be5fd085;x=962d19517545c0b1",
  "_PID": "251476",
  "_MACHINE_ID": "38786fbba498f16c66a525b6642cf555",
  "__REALTIME_TIMESTAMP": "1719295782408325",
  "__MONOTONIC_TIMESTAMP": "91480181414",
  "_SOURCE_REALTIME_TIMESTAMP": "1719295782408217",
  "_BOOT_ID": "92f3c733ee9a41e6a48eb22e1b60d6b5",
  "_CAP_EFFECTIVE": "0",
  "_HOSTNAME": "DESKTOP-D0BLHPQ",
  "SYSLOG_IDENTIFIER": "opentelemetry-journal-exporter",
  "OTEL_NAME": "event opentelemetry-journald-logs/examples/basic-logs.rs:28",
  "PRIORITY": "6",
  "OTEL_EVENT_ID": "1234",
  "_GID": "1001"
}
```

### Example Output (with JSON):

```json
{
  "SYSLOG_IDENTIFIER": "opentelemetry-journal-exporter",
  "_UID": "1000",
  "_HOSTNAME": "DESKTOP-D0BLHPQ",
  "_PID": "251979",
  "MESSAGE": "{\"message\":\"my test message\",\"OTEL_NAME\":\"event opentelemetry-journald-logs/examples/basic-logs.rs:28\",\"OTEL_EVENT_ID\":\"1234\",\"OTEL_USER_ID\":\"5678\"}",
  "_MACHINE_ID": "38786fbba498f16c66a525b6642cf555",
  "__MONOTONIC_TIMESTAMP": "91646562711",
  "_COMM": "basic-logs",
  "_TRANSPORT": "journal",
  "PRIORITY": "6",
  "_GID": "1001",
  "_SOURCE_REALTIME_TIMESTAMP": "1719295948789586",
  "_CAP_EFFECTIVE": "0",
  "_BOOT_ID": "92f3c733ee9a41e6a48eb22e1b60d6b5",
  "__CURSOR": "s=b6ec4d2d7b2f458e92a4935c7915a430;i=23f1a;b=92f3c733ee9a41e6a48eb22e1b60d6b5;m=15568f9197;t=61bb0c84a9776;x=8ee7deaebbd340de",
  "__REALTIME_TIMESTAMP": "1719295948789622"
}
```
