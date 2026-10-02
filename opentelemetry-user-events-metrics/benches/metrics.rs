/*
    Benchmarks a full collect + export cycle for the user_events metrics
    exporter.

    Each iteration records `series` data points and then forces a flush, so the
    measurement covers aggregation, OTLP serialization and the tracepoint
    writes. Recording cost is identical across implementations, so the numbers
    are meaningful when comparing two revisions of the exporter (for example
    one-event-per-data-point versus batched writes).

    IMPORTANT: the exporter short-circuits when the tracepoint has no listener,
    in which case nothing is serialized and the numbers only reflect collection.
    Run on Linux with the tracepoint enabled to measure the export path:

      sudo -E ~/.cargo/bin/cargo bench --bench metrics
      # in another shell, once the tracepoint is registered:
      echo 1 | sudo tee /sys/kernel/tracing/events/user_events/otlp_metrics/enable

    To compare against another revision:

      git checkout main     && cargo bench --bench metrics -- --save-baseline before
      git checkout <branch> && cargo bench --bench metrics -- --baseline before
*/

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use opentelemetry::metrics::MeterProvider;
use opentelemetry::KeyValue;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::Resource;
use opentelemetry_user_events_metrics::MetricsExporter;

/// A deliberately small resource, matching what production agents carry. The
/// envelope it produces is what batching amortizes away.
fn resource() -> Resource {
    Resource::builder_empty()
        .with_attributes([
            KeyValue::new("service.name", "bench-ingest"),
            KeyValue::new(
                "service.instance.id",
                "9f1c2b7e-4a3d-4c11-9b02-7e5f8a1d3c44",
            ),
            KeyValue::new("cloud.region", "eastus2"),
            KeyValue::new("host.name", "node-bench-0731"),
        ])
        .build()
}

/// Attribute sets for `count` dimensions. The high-cardinality `partition` key
/// comes first so every dimension count still yields distinct series.
fn dims(i: usize, count: usize) -> Vec<KeyValue> {
    let all = [
        KeyValue::new("partition", format!("p{i:04}")),
        KeyValue::new("operation", "GetBlobProperties"),
        KeyValue::new("status_code", "200"),
        KeyValue::new("client_version", "2024-11-04"),
        KeyValue::new("protocol", "https"),
    ];
    all.into_iter().take(count).collect()
}

fn provider() -> SdkMeterProvider {
    SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(MetricsExporter::new()).build())
        .with_resource(resource())
        .build()
}

fn bench_counter(c: &mut Criterion) {
    let mut group = c.benchmark_group("export_cycle_counter");
    for series in [100usize, 1000] {
        for n_dims in [2usize, 5] {
            group.throughput(Throughput::Elements(series as u64));
            group.bench_with_input(
                BenchmarkId::new(format!("{n_dims}_dims"), series),
                &series,
                |b, &series| {
                    let provider = provider();
                    let counter = provider
                        .meter("bench")
                        .u64_counter("bench.requests")
                        .build();
                    let attrs: Vec<Vec<KeyValue>> = (0..series).map(|i| dims(i, n_dims)).collect();
                    b.iter(|| {
                        for a in &attrs {
                            counter.add(1, a);
                        }
                        let _ = provider.force_flush();
                    });
                    let _ = provider.shutdown();
                },
            );
        }
    }
    group.finish();
}

fn bench_histogram(c: &mut Criterion) {
    let mut group = c.benchmark_group("export_cycle_histogram");
    for series in [100usize, 1000] {
        group.throughput(Throughput::Elements(series as u64));
        group.bench_with_input(BenchmarkId::new("3_dims", series), &series, |b, &series| {
            let provider = provider();
            let hist = provider
                .meter("bench")
                .f64_histogram("bench.latency")
                .build();
            let attrs: Vec<Vec<KeyValue>> = (0..series).map(|i| dims(i, 3)).collect();
            b.iter(|| {
                for a in &attrs {
                    hist.record(12.5, a);
                }
                let _ = provider.force_flush();
            });
            let _ = provider.shutdown();
        });
    }
    group.finish();
}

criterion_group!(benches, bench_counter, bench_histogram);
criterion_main!(benches);
