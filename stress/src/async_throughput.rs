use futures::stream::{FuturesUnordered, StreamExt};
use num_format::{Locale, ToFormattedString};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

// # AsyncThroughputTest
//
// Measures the throughput and error rate of a user-provided asynchronous function under configurable load.
//
// ## What this measures
//
// - Number of successful and failed operations performed by an async function.
// - Throughput (operations per second), reported live and as a final average.
// - Success rate and error count during the test.
//
// ## How it works
//
// - Launches a configurable number of async worker tasks, each maintaining a set number of concurrent operations.
// - As each operation completes, a new one is started to maintain steady concurrency.
// - Successes and errors are tracked using atomic counters.
// - A separate reporter task prints throughput and errors every second.
// - The test can be stopped gracefully with Ctrl+C, allowing in-flight operations to finish.
//
// ## Typical use-cases
//
// - Load testing and benchmarking async functions, network clients, or service endpoints.
// - Observing how throughput and error rates change under varying concurrency settings.
//
pub struct AsyncThroughputTest {
    stats: Arc<SimpleStats>,
    stop: Arc<AtomicBool>,
}

struct SimpleStats {
    success_count: Arc<AtomicU64>,
    error_count: Arc<AtomicU64>,
}

impl SimpleStats {
    fn new() -> Self {
        Self {
            success_count: Arc::new(AtomicU64::new(0)),
            error_count: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl AsyncThroughputTest {
    pub fn new() -> Self {
        Self {
            stats: Arc::new(SimpleStats::new()),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Run the throughput test with the given async function
    ///
    /// # Arguments
    /// * `name` - Test name for display
    /// * `workers` - Number of worker tasks
    /// * `concurrency_per_worker` - Number of concurrent operations per worker
    /// * `test_fn` - The async function to test
    pub async fn run<F, Fut, E>(
        self,
        name: &str,
        workers: usize,
        concurrency_per_worker: usize,
        test_fn: F,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), E>> + Send + 'static,
        E: std::error::Error + Send + Sync + 'static,
    {
        println!("Async Throughput Test: {}", name);
        println!(
            "Workers: {} | Concurrency/worker: {}",
            workers, concurrency_per_worker
        );
        println!("Total max concurrent: {}", workers * concurrency_per_worker);
        println!("Press Ctrl+C to stop\n");

        let test_fn = Arc::new(test_fn);
        let test_start = Instant::now();

        // Ctrl+C handler
        {
            let stop: Arc<AtomicBool> = self.stop.clone();
            tokio::spawn(async move {
                tokio::signal::ctrl_c().await.ok();
                println!("\n\nGraceful shutdown initiated...");
                stop.store(true, Ordering::SeqCst);
            });
        }

        // Stats reporter
        let reporter = self.spawn_reporter();

        // Worker tasks
        let mut worker_handles = Vec::new();
        for worker_id in 0..workers {
            let handle = self.spawn_worker(worker_id, concurrency_per_worker, test_fn.clone());
            worker_handles.push(handle);
        }

        // Wait for all workers
        let worker_results = futures::future::join_all(worker_handles).await;

        // Stop reporter
        reporter.abort();

        // Print final stats
        self.print_final_stats(test_start.elapsed(), worker_results)
            .await;

        Ok(())
    }

    fn spawn_reporter(&self) -> tokio::task::JoinHandle<()> {
        let stats = self.stats.clone();
        let stop = self.stop.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            let mut last_success = 0u64;

            loop {
                interval.tick().await;

                if stop.load(Ordering::SeqCst) {
                    break;
                }

                let success = stats.success_count.load(Ordering::Relaxed);
                let errors = stats.error_count.load(Ordering::Relaxed);
                let success_delta = success - last_success;
                if success_delta > 0 || errors > 0 {
                    let throughput = success_delta;

                    println!(
                        "Throughput: {:>7}/s | Total: {:>8} | Errors: {}",
                        throughput.to_formatted_string(&Locale::en),
                        success.to_formatted_string(&Locale::en),
                        errors
                    );
                }

                last_success = success;
            }
        })
    }

    fn spawn_worker<F, Fut, E>(
        &self,
        worker_id: usize,
        concurrency: usize,
        test_fn: Arc<F>,
    ) -> tokio::task::JoinHandle<(usize, u64, u64)>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), E>> + Send + 'static,
        E: std::error::Error + Send + Sync + 'static,
    {
        let stats = self.stats.clone();
        let stop = self.stop.clone();

        tokio::spawn(async move {
            let mut futures = FuturesUnordered::new();
            let mut local_success = 0u64;
            let mut local_errors = 0u64;

            // Main loop
            while !stop.load(Ordering::SeqCst) {
                // Keep pipeline full
                while futures.len() < concurrency && !stop.load(Ordering::SeqCst) {
                    let test_fn = test_fn.clone();
                    futures.push(async move {
                        let result = test_fn().await;
                        result.is_ok()
                    });
                }

                // Process completed
                if let Some(success) = futures.next().await {
                    if success {
                        local_success += 1;
                        stats.success_count.fetch_add(1, Ordering::Relaxed); // LIVE UPDATE
                    } else {
                        local_errors += 1;
                        stats.error_count.fetch_add(1, Ordering::Relaxed); // LIVE UPDATE
                    }
                }
            }

            // Drain remaining
            println!(
                "Worker {} draining {} requests...",
                worker_id,
                futures.len()
            );
            while let Some(success) = futures.next().await {
                if success {
                    local_success += 1;
                } else {
                    local_errors += 1;
                }
            }

            (worker_id, local_success, local_errors)
        })
    }

    async fn print_final_stats(
        &self,
        elapsed: Duration,
        _worker_results: Vec<Result<(usize, u64, u64), tokio::task::JoinError>>,
    ) {
        let success = self.stats.success_count.load(Ordering::Relaxed);
        let errors = self.stats.error_count.load(Ordering::Relaxed);
        let total = success + errors;

        println!("\n╔══════════════════════════════════════════════════════╗");
        println!("║                 FINAL RESULTS                        ║");
        println!("╠══════════════════════════════════════════════════════╣");

        println!(
            "║ Duration:        {:<35} ║",
            format!("{:.2}s", elapsed.as_secs_f64())
        );
        println!(
            "║ Total Ops:       {:<35} ║",
            total.to_formatted_string(&Locale::en)
        );
        println!(
            "║ Successful:      {:<35} ║",
            success.to_formatted_string(&Locale::en)
        );
        println!(
            "║ Failed:          {:<35} ║",
            errors.to_formatted_string(&Locale::en)
        );

        if total > 0 {
            println!(
                "║ Success Rate:    {:<35} ║",
                format!("{:.2}%", (success as f64 / total as f64) * 100.0)
            );
            println!(
                "║ Throughput:      {:<35} ║",
                format!("{} ops/s", (success as f64 / elapsed.as_secs_f64()) as u64)
            );
        }

        println!("╚══════════════════════════════════════════════════════╝");
    }
}
