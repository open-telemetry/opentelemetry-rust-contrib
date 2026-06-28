# OpenTelemetry Metric Exporter for Linux user_events

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

| Status        |           |
| ------------- |-----------|
| Stability     | beta      |
| Owners        | [Cijo Thomas](https://github.com/cijothomas), [Lalit Kumar Bhasin](https://github.com/lalitb) |

This crate contains a Metric Exporter to export metrics to Linux
[user_events](https://docs.kernel.org/trace/user_events.html), which is a
solution for user process tracing, similar to ETW (Event Tracing for Windows) on
Windows. It builds on top of the Linux Tracepoints, and so allows user processes
to create events and trace data that can be viewed via existing tools like
ftrace and perf.

This kernel feature is supported started in Linux kernel 5.18 onwards. The feature enables

- A faster path for tracing from user mode application utilizing kernel mode memory address space.
- User processes can now export telemetry events only when it is useful i.e, when the registered set of tracepoint events are enabled.

 This user_events exporter enables applications to use OpenTelemetry API to capture the telemetry events, and write to user_events subsystem. From user_events, the events can be

- Captured by the agents running locally, and listening for specific events within user_events subsystem.
- Or real-time monitoring using local Linux tool like [perf](https://perf.wiki.kernel.org/index.php/Main_Page) or ftrace.

[![Crates.io: opentelemetry-user-events-metrics](https://img.shields.io/crates/v/opentelemetry-user-events-metrics.svg)](https://crates.io/crates/opentelemetry-user-events-metrics)
[![Documentation](https://docs.rs/opentelemetry-user-events-metrics/badge.svg)](https://docs.rs/opentelemetry-user-events-metrics)
[![Slack](https://img.shields.io/badge/slack-@cncf/otel/rust-brightgreen.svg?logo=slack)](https://cloud-native.slack.com/archives/C03GDP0H023)

## OpenTelemetry Overview

OpenTelemetry is an Observability framework and toolkit designed to create and
manage telemetry data such as traces, metrics, and logs. OpenTelemetry is
vendor- and tool-agnostic, meaning that it can be used with a broad variety of
Observability backends, including open source tools like [Jaeger] and
[Prometheus], as well as commercial offerings.

OpenTelemetry is *not* an observability backend like Jaeger, Prometheus, or other
commercial vendors. OpenTelemetry is focused on the generation, collection,
management, and export of telemetry. A major goal of OpenTelemetry is that you
can easily instrument your applications or systems, no matter their language,
infrastructure, or runtime environment. Crucially, the storage and visualization
of telemetry is intentionally left to other tools.

## Capture and Decode Locally with perf

You can validate this exporter on any Linux 6.4+ host with `user_events`
enabled, using `perf` to capture the tracepoint and a small Python script to
decode the payload into OpenTelemetry protobuf.

> For a full ready-to-use Ubuntu VM setup (cloud-init, Rust, helper scripts),
> see [local-setup/README.md](./local-setup/README.md). The steps below
> reproduce only the minimum needed to capture and decode events.

### One-time setup

1. Install `perf-decode` from the [LinuxTracepoints](https://github.com/microsoft/LinuxTracepoints)
   project (decodes `user_events` payloads recorded by `perf` into JSON):

   ```bash
   sudo apt-get install -y build-essential cmake git
   git clone https://github.com/microsoft/LinuxTracepoints
   cd LinuxTracepoints && mkdir build && cd build && cmake .. && make
   sudo cp bin/perf-decode /usr/local/bin/
   ```

2. Install the Python OpenTelemetry proto bindings used by the decoder script:

   ```bash
   python3 -m venv ~/userevents-env
   source ~/userevents-env/bin/activate
   pip install opentelemetry-proto
   ```

3. Save the following helper as `decrypt_python.py` (parses each captured
   buffer as an `ExportMetricsServiceRequest`):

   ```python
   import sys, json
   from opentelemetry.proto.collector.metrics.v1.metrics_service_pb2 import ExportMetricsServiceRequest

   if len(sys.argv) < 2:
       print("Usage: python3 decrypt_python.py <perf.json>")
       sys.exit(1)

   with open(sys.argv[1], 'r', encoding='utf-8-sig') as f:
       data = json.load(f)

   for key in data:
       for item in data[key]:
           req = ExportMetricsServiceRequest()
           req.ParseFromString(bytes(item['buffer']))
           print(req)
   ```

### Capture and decode

1. Start `perf` capture in one shell (keep it running):

   ```bash
   sudo perf record -e user_events:otlp_metrics
   ```

2. In a second shell, build and run the example so it emits metrics to the
   tracepoint:

   ```bash
   cd opentelemetry-rust-contrib/opentelemetry-user-events-metrics/
   cargo build --example basic-metrics --all-features
   sudo ../target/debug/examples/basic-metrics
   ```

3. Stop `perf` (`Ctrl+C`) and decode the capture to JSON:

   ```bash
   sudo chmod uog+r ./perf.data
   perf-decode ./perf.data > perf.json
   ```

   `perf.json` should contain entries like:

   ```json
   { "n": "user_events:otlp_metrics", "protocol": 0, "version": "v0.19.00", "buffer": [ ... ] }
   ```

4. Convert the decoded buffers into OpenTelemetry protobuf text:

   ```bash
   source ~/userevents-env/bin/activate
   python3 decrypt_python.py perf.json
   ```

   The output is the `ExportMetricsServiceRequest` for each captured event,
   including `resource_metrics`, scope, metric name, attributes, and data
   points emitted by the example.
