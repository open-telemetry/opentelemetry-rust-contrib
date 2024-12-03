//! run with `$ cargo bench --bench etw -- --exact <test_name>` to run specific test for logs
//! So to run test named "fibonacci" you would run `$ cargo bench --bench etw -- --exact fibonacci`
//! To run all tests for logs you would run `$ cargo bench --bench etw`
//!
/*
The benchmark results:
criterion = "0.5.1"
OS:
Hardware:
RAM:
| Test                           | Average time|
|--------------------------------|-------------|
|                                |             |
*/

use opentelemetry_etw_metrics::etw::write;
use criterion::{criterion_group, criterion_main, Criterion};

fn write_event() {
    let buffer = "This is a test buffer".as_bytes();
    write(buffer);
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("write_event", |b| b.iter(|| write_event()));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
