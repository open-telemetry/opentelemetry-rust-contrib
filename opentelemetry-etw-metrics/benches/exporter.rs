//! run with `$ cargo bench --bench exporter -- --exact <test_name>` to run specific test for logs
//! So to run test named "fibonacci" you would run `$ cargo bench --bench exporter -- --exact fibonacci`
//! To run all tests for logs you would run `$ cargo bench --bench exporter`
//!
/*
The benchmark results:
criterion = "0.5.1"
OS: Windows 11 Enterprise N, 23H2, Build 22631.4460
Hardware: Intel(R) Xeon(R) Platinum 8370C CPU @ 2.80GHz   2.79 GHz, 16vCPUs
RAM: 64.0 GB
| Test                           | Average time|
|--------------------------------|-------------|
| exporter                       |  22.203 ms  |
*/

use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_etw_metrics::MetricsExporter;

use opentelemetry_proto::tonic::resource;
use opentelemetry_sdk::{
    metrics::{
        data::{Metric, ResourceMetrics, ScopeMetrics, Sum, SumDataPoint},
        exporter::PushMetricExporter,
        Temporality,
    },
    Resource,
};

use criterion::{criterion_group, criterion_main, Criterion};

async fn export(exporter: &MetricsExporter, resource_metrics: &mut ResourceMetrics) {
    exporter.export(resource_metrics).await.unwrap();
}

// fn create_resource_metrics() -> ResourceMetrics {
//     // Metric does not implement clone so this helper function is used to create a metric
//     fn create_metric() -> Metric {
//         let data_point = SumDataPoint {
//             attributes: vec![KeyValue::new("datapoint key", "datapoint value")],
//             value: 1.0_f64,
//             exemplars: vec![],
//         };

//         let sum: Sum<f64> = Sum {
//             data_points: vec![data_point.clone(); 2_000],
//             temporality: Temporality::Delta,
//             start_time: std::time::SystemTime::now(),
//             time: std::time::SystemTime::now(),
//             is_monotonic: true,
//         };

//         Metric {
//             name: "metric_name".into(),
//             description: "metric description".into(),
//             unit: "metric unit".into(),
//             data: Box::new(sum),
//         }
//     }

//     ResourceMetrics {
//         resource: Resource::builder()
//             .with_attributes(vec![KeyValue::new("service.name", "my-service")])
//             .build(),
//         scope_metrics: vec![ScopeMetrics {
//             scope: InstrumentationScope::default(),
//             metrics: vec![
//                 create_metric(),
//                 create_metric(),
//                 create_metric(),
//                 create_metric(),
//                 create_metric(),
//                 create_metric(),
//                 create_metric(),
//                 create_metric(),
//                 create_metric(),
//                 create_metric(),
//             ],
//         }],
//     }
// }

fn criterion_benchmark(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();

    c.bench_function("export", move |b| {
        b.iter_custom(|iters| {
            let exporter = MetricsExporter::new();

            // TODO: Fix this once we can use data without aggregation.
            // let mut resource_metrics = create_resource_metrics();
            let mut resource_metrics = ResourceMetrics::default();

            let start = std::time::Instant::now();

            for _i in 0..iters {
                runtime.block_on(async {
                    export(&exporter, &mut resource_metrics).await;
                });
            }
            start.elapsed()
        })
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
