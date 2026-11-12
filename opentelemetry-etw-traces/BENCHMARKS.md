# Benchmarks

Run from the repository root on Windows. The benchmark implementation and fixtures
are in [benches/traces.rs](benches/traces.rs). Non-Windows runs skip measurement.

## Run without a listener

The default benchmarks measure span lifecycle with the ETW provider disabled:

```powershell
$env:OTEL_ETW_BENCH_MODE = "disabled"
cargo bench -p opentelemetry-etw-traces --bench traces
```

For serialization and SDK-only measurements, enable the additional benchmark groups.
The Geneva feature adds the experimental flat layout alongside the default nested layout:

```powershell
cargo bench -p opentelemetry-etw-traces --bench traces --features bench-internals,experimental-geneva-mapping -- 'Serialization|RecordingLifecycle'
```

## Run with ETW capture

Use an elevated PowerShell. This starts a file-mode listener, runs the enabled
end-to-end cases for both layouts, and stops capture afterward:

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
    cargo bench -p opentelemetry-etw-traces --bench traces --features bench-internals,experimental-geneva-mapping -- '/EndToEnd/'
    if ($LASTEXITCODE -ne 0) { throw "ETW benchmark failed" }
} finally {
    logman stop $session -ets
    $env:OTEL_ETW_BENCH_MODE = $previousMode
}
Write-Host "Capture: $etl"
```

Replace the filter with `/Export/` or `/WriteOnly/` to measure those stages.
Keep the listener running throughout measurement. Selected enabled cases wait
up to five seconds for it; setup and listener checks are outside timing.

Inspect the ETL with PerfView or another ETW decoder to confirm `Span` events and
check for lost events or buffers. Discard lossy captures. If necessary, increase
`-nb` and rerun both revisions with the same settings. The reference runs below
used `-nb 8192 16384`, allowing 512 MiB to 1 GiB of ETW buffers.

## Choose a benchmark

The additional groups require `bench-internals`. Use `-- --list` to list cases.

| Group or filter | What it measures |
| --- | --- |
| `SimpleSpan`, `ScopedSpan` | Original nested-layout span lifecycle cases. Available without extra features. |
| `RecordingLifecycle` | SDK span creation, context handling, and cleanup, with no exporter. |
| `Serialization` | Encoding prebuilt span data into a reused buffer; no ETW write. |
| `/Export/` | Exporter enabled check, serialization, and ETW write when enabled. |
| `/WriteOnly/` | The complete `EventBuilder::write()` method on a prepared event. |
| `/EndToEnd/` | SDK span lifecycle, synchronous export, and cleanup. |

Cases cover zero, three mixed, and six string attributes, plus a parent/error
case. `MixedMapped` adds mapped and custom attributes for serialization and
enabled export. See the source for exact inputs.

Stage timings use prebuilt data and warm buffers; end-to-end timings include
real span creation. They are independent measurements: do not add or subtract
them to derive other costs. Disabled runs do not measure enabled ETW delivery,
and a successful write does not imply disk persistence or backend delivery.

## Reference results

These historical observations from 2026-11-04 give a sense of scale. Values are
microseconds per span, averaged from two per-process Criterion estimates.

| Case | SDK-only | Nested end-to-end | Flat Geneva end-to-end |
| --- | --- | --- | --- |
| 0 attributes | 0.343 | 2.111 | 1.830 |
| 3 mixed attributes | 0.412 | 2.191 | 2.309 |
| 6 string attributes | 0.446 | 2.333 | 2.209 |
| Parent/error | 0.411 | 2.378 | 2.290 |

Measured on a Windows 11 Enterprise VM (10.0.26300), AMD EPYC 7763,
8 guest cores / 16 logical CPUs, 63.95 GiB RAM, with one producer and no affinity
override. The build used Rust 1.99.0, Criterion 0.8.2, default crate features plus
`bench-internals,experimental-geneva-mapping`, and the working tree based on
`47b62c7` with the dual-layout and lifecycle additions, before result comments.

Each process used the filter `RecordingLifecycle|/EndToEnd/` with
`--bench --warm-up-time 1 --measurement-time 2 --noplot` and 100 samples.
Capture used keyword `0x1`, level `5`, and `-bs 64 -nb 8192 16384`; accepted
captures reported zero loss. Layout order was fixed and timings were noisy,
so these values are not regression thresholds or evidence of a faster layout.
Earlier stage results and full run records remain in Git history.

For new comparisons, record the commit, toolchain, command, machine, and capture
settings together. Keep them consistent across revisions and repeat in fresh
processes; capture I/O and machine load can materially affect enabled results.
