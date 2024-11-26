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

use criterion::{criterion_group, criterion_main, Criterion};

fn fibonacci(n: u64) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("fib 20", |b| b.iter(|| fibonacci(20)));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
