![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

# OpenTelemetry user_events Exporter

[![Crates.io: opentelemetry-user-events-metrics](https://img.shields.io/crates/v/opentelemetry-user-events-metrics.svg)](https://crates.io/crates/opentelemetry-user-events-metrics)

This crate contains OpenTelemetry metrics exporter to
[user_events](https://docs.kernel.org/trace/user_events.html) , a Linux solution
for user process tracing, similar to ETW (Event Tracing for Windows) on Windows.
It builds on top of the Linux Tracepoints, and so allows user processes to
create events and trace data that can be viewed via existing tools like ftrace
and perf.

This kernel feature is supported started in Linux kernel 5.18 onwards. The feature enables

- A faster path for tracing from user mode application utilizing kernel mode memory address space.
- User processes can now export telemetry events only when it is useful i.e, when the registered set of tracepoint events are enabled.

 This user_events exporter enables applications to use OpenTelemetry APIs to capture telemetry events and write to the user_events subsystem. From user_events, the events can be
  
- Captured by agents running locally and listening for specific events within the user_events subsystem.
- Real-time monitoring using local Linux tools like [perf](https://perf.wiki.kernel.org/index.php/Main_Page) or ftrace.
