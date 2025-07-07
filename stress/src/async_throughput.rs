use futures::stream::{self, StreamExt};
use std::future::Future;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

/// Statistics collected during the test
#[derive(Clone)]
pub struct ThroughputStats {
    pub completed_ops: u64,
    pub successful_ops: u64,
    pub duration: Duration,
    pub throughput: f64,
}

impl ThroughputStats {
    pub fn print(&self, label: &str) {
        let success_percentage = if self.completed_ops > 0 {
            (self.successful_ops as f64 / self.completed_ops as f64) * 100.0
        } else {
            0.0
        };

        println!(
            "{}: {} ops ({} successful, {:.1}%) in {:.2}s = {:.2} ops/sec",
            label,
            self.completed_ops,
            self.successful_ops,
            success_percentage,
            self.duration.as_secs_f64(),
            self.throughput
        );
    }
}

/// Configuration for the throughput test
pub struct ThroughputConfig {
    /// Number of concurrent operations
    pub concurrency: usize,
    /// Reporting interval for continuous tests
    pub report_interval: Duration,
    /// Optional target number of operations (None for continuous)
    pub target_ops: Option<usize>,
    /// Whether to use task spawning (for multi-thread runtime)
    pub use_spawn: bool,
}

impl Default for ThroughputConfig {
    fn default() -> Self {
        Self {
            concurrency: 100,
            report_interval: Duration::from_secs(5),
            target_ops: None,
            use_spawn: true,
        }
    }
}

/// Generic stream-based throughput tester
pub struct ThroughputTest;

impl ThroughputTest {
    /// Run a continuous throughput test (until interrupted)
    pub async fn run_continuous<F, Fut, T, E>(
        name: &str,
        config: ThroughputConfig,
        operation_factory: F,
    ) where
        F: Fn() -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
        T: Send + 'static,
        E: std::fmt::Display + Send + 'static,
    {
        println!(
            "Testing {} with concurrency level: {}",
            name, config.concurrency
        );

        let completed_ops = Arc::new(AtomicU64::new(0));
        let successful_ops = Arc::new(AtomicU64::new(0));

        // Clone for reporting thread
        let completed_clone = Arc::clone(&completed_ops);
        let successful_clone = Arc::clone(&successful_ops);
        let start_time = Instant::now();
        let report_interval = config.report_interval;

        // Spawn reporting thread
        let _reporting_handle = std::thread::spawn(move || {
            #[cfg(feature = "stats")]
            let pid = sysinfo::Pid::from(std::process::id() as usize);
            #[cfg(feature = "stats")]
            let mut system = sysinfo::System::new_all();

            loop {
                std::thread::sleep(report_interval);
                let ops = completed_clone.load(Ordering::Relaxed);
                let success = successful_clone.load(Ordering::Relaxed);
                let elapsed = start_time.elapsed();
                let throughput = ops as f64 / elapsed.as_secs_f64();

                let success_percentage = if ops > 0 {
                    (success as f64 / ops as f64) * 100.0
                } else {
                    0.0
                };

                println!(
                    "Progress: {} ops completed ({} successful, {:.1}%) in {:.2}s = {:.2} ops/sec",
                    ops,
                    success,
                    success_percentage,
                    elapsed.as_secs_f64(),
                    throughput
                );
                #[cfg(feature = "stats")]
                {
                    system.refresh_all();
                    if let Some(process) = system.process(pid) {
                        println!(
                            "Memory: {:.2} MB | CPU: {:.1}% | Virtual Memory: {:.2} MB",
                            process.memory() as f64 / (1024.0 * 1024.0),
                            process.cpu_usage(),
                            process.virtual_memory() as f64 / (1024.0 * 1024.0)
                        );
                    } else {
                        println!("Process not found");
                    }
                }
            }
        });

        // Create infinite stream of operations
        if config.use_spawn {
            // Create infinite stream of operations with spawning
            let mut operation_stream = stream::iter(0..)
                .map(|_| tokio::task::spawn(operation_factory()))
                .buffer_unordered(config.concurrency);

            // Process stream until interrupted
            while let Some(result) = operation_stream.next().await {
                completed_ops.fetch_add(1, Ordering::Relaxed);
                match result {
                    Ok(Ok(_)) => {
                        successful_ops.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(Err(e)) => {
                        eprintln!("Operation failed: {e}");
                    }
                    Err(e) => {
                        eprintln!("Task join error: {e}");
                    }
                }
            }
        } else {
            // Create infinite stream of operations without spawning
            let mut operation_stream = stream::iter(0..)
                .map(|_| operation_factory())
                .buffer_unordered(config.concurrency);

            // Process stream until interrupted
            while let Some(result) = operation_stream.next().await {
                completed_ops.fetch_add(1, Ordering::Relaxed);
                match result {
                    Ok(_) => {
                        successful_ops.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        eprintln!("Operation failed: {e}");
                    }
                }
            }
        }
    }

    /// Run a fixed-target throughput test
    pub async fn run_fixed<F, Fut, T, E>(
        name: &str,
        config: ThroughputConfig,
        operation_factory: F,
    ) -> ThroughputStats
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
        T: Send + 'static,
        E: std::fmt::Display + Send + 'static,
    {
        let target = config
            .target_ops
            .expect("Target ops must be specified for fixed test");
        let config_concurrency = config.concurrency;
        println!(
            "Testing {name} to run {target} operations with concurrency level: {config_concurrency}"
        );

        let start_time = Instant::now();
        let mut completed_ops = 0;
        let mut successful_ops = 0;

        if config.use_spawn {
            let mut operation_stream = stream::iter(0..target)
                .map(|_| tokio::task::spawn(operation_factory()))
                .buffer_unordered(config.concurrency);

            while let Some(result) = operation_stream.next().await {
                completed_ops += 1;
                match result {
                    Ok(Ok(_)) => successful_ops += 1,
                    Ok(Err(e)) => {
                        eprintln!("Operation failed: {e}");
                    }
                    Err(e) => {
                        eprintln!("Task join error: {e}");
                    }
                }
            }
        } else {
            let mut operation_stream = stream::iter(0..target)
                .map(|_| operation_factory())
                .buffer_unordered(config.concurrency);

            while let Some(result) = operation_stream.next().await {
                completed_ops += 1;
                match result {
                    Ok(_) => successful_ops += 1,
                    Err(e) => {
                        eprintln!("Operation failed: {e}");
                    }
                }
            }
        }

        let duration = start_time.elapsed();
        ThroughputStats {
            completed_ops,
            successful_ops,
            duration,
            throughput: completed_ops as f64 / duration.as_secs_f64(),
        }
    }

    /// Run comparison tests with different concurrency levels
    pub async fn run_comparison<F, Fut, T, E>(
        name: &str,
        concurrency_levels: &[usize],
        target_ops: usize,
        operation_factory: F,
    ) -> Vec<(usize, ThroughputStats)>
    where
        F: Fn() -> Fut + Send + Sync + 'static + Clone,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
        T: Send + 'static,
        E: std::fmt::Display + Send + 'static,
    {
        println!("Concurrency Level Comparison for {name}:");
        let mut results = Vec::new();

        for &concurrency in concurrency_levels {
            let config = ThroughputConfig {
                concurrency,
                target_ops: Some(target_ops),
                ..Default::default()
            };

            let stats = Self::run_fixed(name, config, operation_factory.clone()).await;
            stats.print(&format!("   Concurrency {concurrency}"));
            results.push((concurrency, stats));
        }

        results
    }
}

/// Example usage module
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    async fn example_async_operation() -> Result<String, std::io::Error> {
        // Simulate some async work
        sleep(Duration::from_millis(10)).await;
        Ok("Success".to_string())
    }

    #[tokio::test]
    async fn test_fixed_throughput() {
        let config = ThroughputConfig {
            concurrency: 50,
            target_ops: Some(1000),
            ..Default::default()
        };

        let stats =
            ThroughputTest::run_fixed("Example Operation", config, example_async_operation).await;

        stats.print("Final Results");
        assert_eq!(stats.completed_ops, 1000);
    }
}
