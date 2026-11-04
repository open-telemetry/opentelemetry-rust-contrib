# OpenTelemetry Trace Exporter for ETW

![OpenTelemetry — An observability framework for cloud-native software.][splash]

[splash]: https://raw.githubusercontent.com/open-telemetry/opentelemetry-rust/main/assets/logo-text.png

| Status        |           |
| ------------- |-----------|
| Stability     | beta      |
| Owners        | [Jelena Vidakovic](https://github.com/jevidako), [Uros Jovanovic](https://github.com/urosjovanovic) |

This crate contains an OpenTelemetry Trace exporter to
[ETW (Event Tracing for Windows)](https://learn.microsoft.com/en-us/windows-hardware/drivers/devtest/event-tracing-for-windows--etw-), a Windows solution
for efficient tracing of kernel or application-defined events, similar to user_events on Linux.
ETW events can be consumed in real-time or from a log file.

ETW events created with this crate can be generated and collected on Windows Vista or later.

This ETW exporter enables applications to use OpenTelemetry APIs to capture span data and write to the ETW subsystem. From ETW, the events can be
captured by agents running locally and listening for specific ETW events.

Spans are encoded following the [Microsoft Common Schema v4.0](https://learn.microsoft.com/en-us/opentelemetry/common-schema) format using
[TraceLogging Dynamic](https://crates.io/crates/tracelogging_dynamic).

[![Crates.io: opentelemetry-etw-traces](https://img.shields.io/crates/v/opentelemetry-etw-traces.svg)](https://crates.io/crates/opentelemetry-etw-traces)
[![Documentation](https://docs.rs/opentelemetry-etw-traces/badge.svg)](https://docs.rs/opentelemetry-etw-traces)
[![Slack](https://img.shields.io/badge/slack-@cncf/otel/rust-brightgreen.svg?logo=slack)](https://cloud-native.slack.com/archives/C03GDP0H023)

## Viewing ETW Traces

Traces exported to ETW can be viewed using tools like `logman`, `perfview` etc.

### Using `logman`

To view the telemetry emitted to ETW you can use [`logman`](https://learn.microsoft.com/windows-server/administration/windows-commands/logman) along with `perfview`.
`logman` will listen to ETW events from the given provider (on this example, `provider_name`) and store them in a `.etl` file.

[`perfview`](https://github.com/microsoft/perfview) will allow you to visualize the events.

Instructions using Powershell:

1. Get the ETW Session Guid for the given provider (on this example `provider_name`):

    ```ps
    $EtwSessionGuid = (new-object System.Diagnostics.Tracing.EventSource("provider_name")).Guid.ToString()
    ```

1. Start Logman session:

    ```ps
    logman create trace OtelETWTracesBasic -o OtelETWTracesBasic.log -p "{$EtwSessionGuid}" -f bincirc -max 1000
    logman start OtelETWTracesBasic
    ```

1. Execute the example:

    ```ps
    cd opentelemetry-etw-traces
    cargo run --example basic
    ```

1. Stop and Remove `logman` session:

    ```ps
    logman stop OtelETWTracesBasic
    logman delete OtelETWTracesBasic
    ```

1. View the events with `perfview`:

    - Download PerfView: [Instructions](https://github.com/microsoft/perfview/blob/main/documentation/Downloading.md), [Releases](https://github.com/Microsoft/perfview/releases).
    - Open PerfView.
    - Go the location of the `.etl` file: `OtelETWTracesBasic.log_000001.etl` and open it.
    - Double-click `Events` in the left-panel.
    - Double-click the `provider_name/Span` in the left-panel.
    - You should see the events in the right-panel.

## Benchmarks

The Windows-only Criterion benchmark has three lifecycle cases:

| Case | Timed work |
| ---- | ---------- |
| `SimpleSpan` | Create a span with six string attributes, end it, and drop it, matching the workload in the user-events trace benchmark. |
| `ScopedSpan/0` | Create a Server span, attach it as the active context, set `Status::Ok`, end it, detach the context, and drop the span. |
| `ScopedSpan/3` | The same scoped lifecycle with string, integer, and boolean attributes. |

All cases use the synchronous ETW processor, an always-on sampler, and a
`service.name` resource of `etw-trace-benchmark`. Provider initialization and
listener checks are outside timing. Scoped attribute inputs are prepared outside
timing; copying them into each span and final cleanup are included. There is no
synthetic application work in the timed loop.

### Without an ETW listener

Run from the repository root on Windows:

```powershell
$env:OTEL_ETW_BENCH_MODE = "disabled"
cargo bench -p opentelemetry-etw-traces --bench traces
```

`disabled` is the default. This measures SDK span lifecycle overhead with the
exporter's disabled-provider fast path, **not serialization or ETW writes**.
Results are named `ETW/disabled/...`.

### With an ETW listener

In an elevated PowerShell, start a file-mode session before running the benchmark.
The provider name must match `OpenTelemetryEtwTraceBenchmark`:

```powershell
$source = [System.Diagnostics.Tracing.EventSource]::new("OpenTelemetryEtwTraceBenchmark")
$providerGuid = $source.Guid.ToString("B")
$source.Dispose()
$session = "OtelEtwTraceBench-$PID"
$etl = Join-Path $env:TEMP "$session.etl"
$previousMode = $env:OTEL_ETW_BENCH_MODE

logman create trace $session -p $providerGuid 0x1 5 -bs 64 -nb 128 512 -o $etl -ets
if ($LASTEXITCODE -ne 0) { throw "Could not start the ETW capture session" }
try {
    $env:OTEL_ETW_BENCH_MODE = "enabled"
    cargo bench -p opentelemetry-etw-traces --bench traces
    if ($LASTEXITCODE -ne 0) { throw "ETW benchmark failed" }
} finally {
    logman stop $session -ets
    $env:OTEL_ETW_BENCH_MODE = $previousMode
}
Write-Host "Capture: $etl"
```

Results are named `ETW/enabled/...`. When the first selected benchmark executes,
its setup waits up to five seconds for the provider to become enabled, outside
timing. Listener-state checks remain before and after each timed batch, including
in disabled mode. Listing (`--list`) and filters matching no benchmarks do not
wait for or check a listener. Do not start or stop listeners during measurement.

The enable check does not prove lossless delivery. Inspect the ETL with PerfView
or another ETW decoder to confirm `Span` events and check for lost events/buffers;
discard lossy captures. Capture I/O, machine load, and buffer configuration affect
enabled results. Keep them consistent when comparing revisions. Non-Windows
runs print a skip message rather than reporting disabled-exporter timings.

These are performance measurements, not tests with fixed timing thresholds.
Similar workloads do not make ETW/Windows timings a controlled comparison against
user-events/Linux results: the operating systems, exporters, hardware, and capture
configurations differ.

### Serialization and exporter-only costs

The opt-in `bench-internals` feature adds paired stage benchmarks using the same
prebuilt `SpanData` and resource:

| Case | Timed work |
| ---- | ---------- |
| `ETW/Serialization/0`, `/3`, `/6` | The production `#[inline]` encoder: reset the reused EventBuilder, build Common Schema metadata and payload, format timestamps and IDs, and encode attributes. No enabled check, TLS lookup, or ETW write. |
| `ETW/enabled/Export/0`, `/3`, `/6` | The real exporter: enabled check, TLS builder borrow, the same serialization, and synchronous ETW write. **Includes serialization**, not write-only. |
| `ETW/disabled/Export/0`, `/3`, `/6` | Only the exporter's disabled-provider fast path; not meaningful as serialization or delivery cost. |

These exclude SDK span creation, sampling, context attachment, status updates,
input allocation, and span cleanup. The fixture is a root Server span named
`BenchmarkOperation`, with `Status::Ok`, fixed nonzero IDs, fixed nanosecond
timestamps, no links/events, and `service.name=etw-trace-benchmark`. `/0` has no
attributes; `/3` has the same string, integer and boolean values as `ScopedSpan/3`;
`/6` has `key1=value1` through `key6=value6`. The six-attribute fixture still uses
Server/Ok, unlike `SimpleSpan`. Both stages reuse buffer capacity; cold
allocation/initialization is not measured. `#[inline]` is a compiler hint.

Run serialization alone without an ETW listener:

```powershell
cargo bench -p opentelemetry-etw-traces --bench traces --features bench-internals -- Serialization
```

For enabled export, start the same ETW session shown above and use:

```powershell
$env:OTEL_ETW_BENCH_MODE = "enabled"
cargo bench -p opentelemetry-etw-traces --bench traces --features bench-internals -- 'Serialization|/Export/'
```

Serialization never waits for a listener. Export checks the actual exporter's
provider before and after each batch, outside timing, and waits only when a
selected enabled-export case first executes. Listing and unmatched filters remain
independent of listener state. The feature exposes only a doc-hidden, unstable
benchmark adapter; the internal resource and exporter remain private. Normal
builds and benchmark defaults are unchanged.

Serialization and enabled export are independently measured, not inferred by
subtracting SDK lifecycle timings. Do not label their difference as a directly
measured ETW-write cost: TLS, enabled checks, compiler placement, and measurement
noise also differ.

#### Optimization comparison (2026-11-04)

The serializer now uses stack-backed lowercase ID encoding and timestamp
formatting, borrows error descriptions, and collects references to the first
occurrence of each well-known attribute while counting custom attributes.
Part B emits those references in the original schema order; Part C still emits
resource attributes first and custom span attributes in input order. Duplicate
well-known attributes retain the original first-occurrence-wins behavior.

The timestamp fast path keeps Chrono's calendar conversion and the original
`AutoSi` fractional widths (0, 3, 6, or 9 digits). Extended years and leap seconds
fall back to the original Chrono formatter. No unsafe Rust, dependencies, global
caches, shared locks, or toolchain changes were introduced. The ordinary root-span
fixtures no longer create the four temporary strings for two IDs and two
timestamps. Parent-ID and error-description changes are covered for correctness,
but their benefit is not measured by the root/`Status::Ok` benchmark fixtures.

The baseline implementation at `97d8baa` and the optimized working tree were
measured in four separate processes, ordered **baseline, optimized, optimized,
baseline**. The table shows arithmetic means of the two Criterion point estimates
per variant, in microseconds per span, not individual-span latency percentiles:

| Attributes | Serialization before | After | Reduction | Enabled export before | After | Reduction |
| ---------- | -------------------- | ----- | --------- | --------------------- | ----- | --------- |
| 0 | 0.594 | 0.392 | 34.0% | 1.537 | 1.413 | 8.0% |
| 3 mixed | 0.730 | 0.447 | 38.7% | 1.675 | 1.435 | 14.4% |
| 6 strings | 0.797 | 0.502 | 37.0% | 2.016 | 1.458 | 27.7% |

**Enabled export includes serialization.** SDK span creation/context/cleanup is
excluded. Serialization improved consistently. Export was noisier: in the second
pair, 0-attribute export was 1.385 us baseline versus 1.458 us optimized, so the
aggregate 8.0% reduction is not an established improvement. These are limited
single-machine results, not regression thresholds.

Per-process point estimates [95% Criterion confidence interval], in microseconds:

| Case | Baseline 1 | Optimized 1 | Optimized 2 | Baseline 2 |
| ---- | ---------- | ----------- | ----------- | ---------- |
| Serialization/0 | 0.591 [0.583, 0.600] | 0.392 [0.389, 0.396] | 0.391 [0.388, 0.395] | 0.596 [0.586, 0.610] |
| Serialization/3 | 0.738 [0.727, 0.752] | 0.448 [0.446, 0.452] | 0.446 [0.445, 0.448] | 0.721 [0.717, 0.727] |
| Serialization/6 | 0.799 [0.794, 0.804] | 0.497 [0.494, 0.501] | 0.507 [0.500, 0.516] | 0.795 [0.791, 0.800] |
| Export/0 | 1.689 [1.533, 1.852] | 1.368 [1.242, 1.503] | 1.458 [1.321, 1.596] | 1.385 [1.280, 1.502] |
| Export/3 | 1.687 [1.551, 1.829] | 1.361 [1.228, 1.501] | 1.508 [1.376, 1.640] | 1.664 [1.533, 1.805] |
| Export/6 | 2.052 [1.897, 2.202] | 1.447 [1.309, 1.594] | 1.469 [1.331, 1.617] | 1.980 [1.821, 2.127] |

Environment was recorded with this comparison: AMD EPYC 7763, 8 guest-visible
cores / 16 logical CPUs, 63.95 GiB RAM, Microsoft VM, Windows 11 Enterprise x64
10.0.26300, rustc 1.99.0, Criterion 0.8.2, OpenTelemetry API/SDK 0.33.1,
Chrono 0.4.45, TraceLogging Dynamic 1.2.4. Each process had one synchronous producer
thread, no affinity override (mask `0xffff`), and the unchanged fixtures above.
Default crate features plus `bench-internals` were used.

Build each revision with the same toolchain/dependencies and preserve its
executable separately. For each of the four runs, start the file-mode session
shown earlier, but use **`-bs 64 -nb 8192 16384`**, then execute:

```powershell
$env:OTEL_ETW_BENCH_MODE = "enabled"
& $exe 'Serialization|/Export/' --bench --warm-up-time 1 --measurement-time 2 --noplot
```

These are CLI overrides, not changes to Criterion defaults: 100 samples and
100,000 resamples remain the defaults; warm-up is 1 s and measurement is 2 s per
case. The capture has 64 KiB buffers, minimum 8192 / maximum 16384 (512 MiB /
1 GiB buffer capacities), keyword `0x1`, level `5`, no PID filter or clock override.
Stop and validate each capture before starting the next process.

Full-default optimized attempts lost events with smaller buffer pools, so those
comparisons were discarded, including their baseline runs. Both variants were
rerun with the same shorter windows and larger buffers; do not compare this table
directly against the older single-run export numbers below. The four accepted
captures contained **26,789,462 provider events**, including warm-up, with zero
reported lost events/buffers. Every event's descriptor, TraceLogging schema bytes,
and payload bytes matched one of the three original pre-optimization fixtures.
Formatting equivalence checks also cover signed years, Chrono limits, leap
seconds/days, pre-epoch dates, fractional precision, and ID hex formatting.

#### Historical stage results (2026-11-04, before optimization)

Microseconds per span, Criterion point estimates with 95% confidence intervals:

| Span attributes | Serialization only | Enabled export, **including serialization** |
| --------------- | ------------------ | ------------------------------------------- |
| 0 | 0.585 [0.582, 0.588] | 1.663 [1.547, 1.780] |
| 3 mixed | 0.732 [0.727, 0.737] | 1.765 [1.671, 1.865] |
| 6 strings | 0.792 [0.789, 0.795] | 2.031 [1.917, 2.149] |

Environment recorded immediately before this run: AMD EPYC 7763, **8 guest-visible
cores / 16 logical CPUs**, 63.95 GiB RAM, Microsoft Virtual Machine, Windows 11
Enterprise x64 10.0.26300 (build 26300). Rust **1.99.0**
(`b940084d7`, LLVM 23.1.1), `x86_64-pc-windows-msvc`, Criterion **0.8.2**,
OpenTelemetry API/SDK **0.33.1**, TraceLogging Dynamic **1.2.4**. Default crate
features plus `bench-internals`; `serde_json` was not enabled for measurement.
The repository toolchain pin and bench profile were unchanged.

This was one process with **one synchronous span-producing thread**, not a
multithread scaling test. No affinity override was applied (inherited process
mask `0xffff`); Windows could migrate the thread among all 16 logical CPUs.
Criterion analysis and ETW collection are outside the producer's timed work;
the measured export call does not wait for disk persistence.

The measured executable was built from `4da9a02` plus the encoder/benchmark
changes in this branch, before the result comments and documentation were added:

```powershell
cargo bench -p opentelemetry-etw-traces --bench traces --features bench-internals --no-run
$env:OTEL_ETW_BENCH_MODE = "enabled"
# With the file-mode ETW session above already running:
& $exe 'Serialization|/Export/' --bench --noplot
```

`$exe` was the resulting `traces-c8ba5eda24759944.exe`, invoked directly.
Its SHA-256 was
`ad2e1c93ba9a7f6345399168c7b46c07ccbc527d562aa565f787644ca8fbf22a`.
Criterion's measurement defaults were used: **100 samples, 3 s warm-up, 5 s
measurement per case, 100,000 resamples, 95% confidence**. Only plot generation
was disabled. Cases ran sequentially in this order: Serialization/0, Export/0,
Serialization/3, Export/3, Serialization/6, Export/6. A preliminary 30-sample,
0.5 s warm-up / 1 s measurement run was noisier and is not included in this table.

Both stages ran while the file-mode ETW session was active, using provider
`{22282217-c59e-57fe-2368-a9ac925aac2a}`, keyword `0x1`, level `5`, 64 KiB buffers,
minimum 128 / maximum 512, and no PID filter or clock override. The session
reported zero lost events, log buffers, or realtime buffers. The separate smoke
capture below is not included in the **14,346,203 provider events** processed by
`tracerpt` from the measured run's ETL (which includes warm-up events). The smoke
capture contained exactly three stage export events; serialization emitted none.
An elevated integration test also compared decoded serialization-only output
(written separately outside timing) against real export for 6, 0, and 3 attributes
using reused buffers. The existing Common Schema integration test was retained;
its fixed startup sleep was replaced with a provider-ready check.

The fixtures' Windows `SystemTime` values are quantized to 100 ns and encoded as
`2023-11-14T22:13:20.123456700Z` / `2023-11-14T22:13:21.123456700Z`. Timestamp/ID
formatting still runs on every serialization. Results are warm, repeated-input
costs, not arbitrary-size or cold-cache costs. The export confidence intervals
remain wider than serialization's; this single VM run is not a regression
threshold or a controlled comparison with the older lifecycle/FFI measurements.

### Preliminary run provenance

The short validation runs on 2026-11-03 used rustc **1.99.0**, matching the
repository toolchain pin. The disabled run used this command from the repository
root:

```powershell
$env:OTEL_ETW_BENCH_MODE = "disabled"
cargo bench -q -p opentelemetry-etw-traces --bench traces -- --sample-size 10 --warm-up-time 0.1 --measurement-time 0.2 --nresamples 1000 --noplot
```

The enabled run invoked the already-built release benchmark
`traces-12444351cfd22eb6.exe` directly (`$exe` in the capture script):

```powershell
$env:OTEL_ETW_BENCH_MODE = "enabled"
& $exe --bench --sample-size 10 --warm-up-time 0.1 --measurement-time 0.2 --nresamples 1000 --noplot
```

These are command-line overrides, not changes to Criterion's default
configuration. The historical lifecycle table in the benchmark source contains
preliminary Criterion point estimates from these short runs, not stable regression
baselines.

The capture script records file-mode ETL capture using
`-p $providerGuid 0x1 5 -bs 64 -nb 128 512 -o $etl -ets`: keyword `0x1`, Verbose
level `5`, 64 KiB buffers, minimum 128 and maximum 512 buffers, and no PID filter.
No CPU-affinity or clock override was specified. The paired smoke capture's ETL
header records Windows NT 10.0, build 26300, 64-bit, and 16 logical processors.

The original run artifacts do **not** establish the CPU model, physical core count,
or whether the machine was virtual or physical. The machine details accompanying
the results were collected afterward, not as part of the measured runs. Record
that environment information alongside future runs for complete per-run provenance.

A later read-only Windows CIM inventory, queried on 2026-11-03, reports an
AMD EPYC 7763 64-Core Processor, 8 guest-visible cores / 16 logical processors,
63.95 GiB RAM, Microsoft Virtual Machine, and Windows 11 Enterprise x64
10.0.26300 (build 26300). The CPU model's "64-Core" designation is not the VM's
allocated core count. This current inventory is also noted at the top of
`benches/traces.rs`; it does not retroactively establish the earlier run environment.

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

[Prometheus]: https://prometheus.io
[Jaeger]: https://www.jaegertracing.io
