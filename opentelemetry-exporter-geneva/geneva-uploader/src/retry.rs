//! Simple retry functionality that can be shared across services

use std::time::Duration;
use tokio::time::sleep;

/// Simple retry configuration
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (0 means no retries)
    pub max_retries: u32,
    /// Fixed delay between retries
    pub delay: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            delay: Duration::from_millis(1000), // 1 second
        }
    }
}

impl RetryConfig {
    /// Create a new retry configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the maximum number of retry attempts
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Set the delay between retries
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

/// Execute a function with retry logic
pub async fn retry_with_config<F, Fut, T, E>(
    config: &RetryConfig,
    operation_name: &str,
    mut operation: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display + Clone,
{
    let max_attempts = config.max_retries + 1; // +1 for the initial attempt
    let mut last_error = None;

    for attempt in 0..max_attempts {
        // Wait before retry (but not before the first attempt)
        if attempt > 0 {
            if !config.delay.is_zero() {
                sleep(config.delay).await;
            }
        }

        match operation().await {
            Ok(result) => {
                // Success! Log retry info if this wasn't the first attempt
                if attempt > 0 {
                    eprintln!(
                        "{} succeeded on attempt {} after retries",
                        operation_name,
                        attempt + 1
                    );
                }
                return Ok(result);
            }
            Err(error) => {
                last_error = Some(error.clone());

                // Only retry if we haven't reached max attempts
                if attempt < max_attempts - 1 {
                    eprintln!(
                        "{} attempt {} failed: {}. Retrying in {:?}...",
                        operation_name,
                        attempt + 1,
                        error,
                        config.delay
                    );
                    continue;
                } else {
                    // We've exhausted all retries
                    eprintln!(
                        "{} failed after {} attempts: {}",
                        operation_name, max_attempts, error
                    );
                    return Err(error);
                }
            }
        }
    }

    // This should never be reached, but just in case
    Err(last_error.unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[derive(Debug, Clone)]
    struct TestError {
        message: String,
        retriable: bool,
    }

    impl std::fmt::Display for TestError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.message)
        }
    }

    impl TestError {
        fn new(message: &str, retriable: bool) -> Self {
            Self {
                message: message.to_string(),
                retriable,
            }
        }

        fn is_retriable(&self) -> bool {
            self.retriable
        }
    }

    #[tokio::test]
    async fn test_retry_config_defaults() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.delay, Duration::from_millis(1000));
    }

    #[tokio::test]
    async fn test_retry_config_builder() {
        let config = RetryConfig::new()
            .with_max_retries(5)
            .with_delay(Duration::from_millis(500));

        assert_eq!(config.max_retries, 5);
        assert_eq!(config.delay, Duration::from_millis(500));
    }

    #[tokio::test]
    async fn test_successful_operation_no_retry() {
        let config = RetryConfig::new().with_max_retries(3);
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result = retry_with_config_and_check(
            &config,
            "test_operation",
            || {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok::<i32, TestError>(42)
                }
            },
            |_| true,
        )
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retriable_error_with_eventual_success() {
        let config = RetryConfig::new()
            .with_max_retries(3)
            .with_delay(Duration::from_millis(1)); // Very short delay for testing

        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result = retry_with_config_and_check(
            &config,
            "test_operation",
            || {
                let count = call_count_clone.clone();
                async move {
                    let current_count = count.fetch_add(1, Ordering::SeqCst);
                    if current_count < 2 {
                        Err(TestError::new("retriable error", true))
                    } else {
                        Ok(42)
                    }
                }
            },
            |error| error.is_retriable(),
        )
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(call_count.load(Ordering::SeqCst), 3); // Failed twice, succeeded on third
    }

    #[tokio::test]
    async fn test_non_retriable_error_no_retry() {
        let config = RetryConfig::new().with_max_retries(3);
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result = retry_with_config_and_check(
            &config,
            "test_operation",
            || {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Err::<i32, TestError>(TestError::new("non-retriable error", false))
                }
            },
            |error| error.is_retriable(),
        )
        .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().message, "non-retriable error");
        assert_eq!(call_count.load(Ordering::SeqCst), 1); // Only called once
    }

    #[tokio::test]
    async fn test_max_retries_exhausted() {
        let config = RetryConfig::new()
            .with_max_retries(2)
            .with_delay(Duration::from_millis(1)); // Very short delay for testing

        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result = retry_with_config_and_check(
            &config,
            "test_operation",
            || {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Err::<i32, TestError>(TestError::new("always fails", true))
                }
            },
            |error| error.is_retriable(),
        )
        .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().message, "always fails");
        assert_eq!(call_count.load(Ordering::SeqCst), 3); // Initial attempt + 2 retries
    }

    #[tokio::test]
    async fn test_zero_retries() {
        let config = RetryConfig::new().with_max_retries(0);
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result = retry_with_config_and_check(
            &config,
            "test_operation",
            || {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Err::<i32, TestError>(TestError::new("error", true))
                }
            },
            |error| error.is_retriable(),
        )
        .await;

        assert!(result.is_err());
        assert_eq!(call_count.load(Ordering::SeqCst), 1); // Only initial attempt
    }

    #[tokio::test]
    async fn test_legacy_retry_function() {
        let config = RetryConfig::new()
            .with_max_retries(2)
            .with_delay(Duration::from_millis(1));

        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result = retry_with_config(&config, "test_operation", || {
            let count = call_count_clone.clone();
            async move {
                let current_count = count.fetch_add(1, Ordering::SeqCst);
                if current_count < 1 {
                    Err(TestError::new("retriable error", true))
                } else {
                    Ok(42)
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(call_count.load(Ordering::SeqCst), 2); // Failed once, succeeded on second
    }
}

/// Execute a function with retry logic that includes retriability checking
pub async fn retry_with_config_and_check<F, Fut, T, E, R>(
    config: &RetryConfig,
    operation_name: &str,
    mut operation: F,
    is_retriable: R,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display + Clone,
    R: Fn(&E) -> bool,
{
    let max_attempts = config.max_retries + 1; // +1 for the initial attempt
    let mut last_error = None;

    for attempt in 0..max_attempts {
        // Wait before retry (but not before the first attempt)
        if attempt > 0 {
            if !config.delay.is_zero() {
                sleep(config.delay).await;
            }
        }

        match operation().await {
            Ok(result) => {
                // Success! Log retry info if this wasn't the first attempt
                if attempt > 0 {
                    eprintln!(
                        "{} succeeded on attempt {} after retries",
                        operation_name,
                        attempt + 1
                    );
                }
                return Ok(result);
            }
            Err(error) => {
                last_error = Some(error.clone());

                // Check if we should retry - only if we haven't reached max attempts AND error is retriable
                if attempt < max_attempts - 1 && is_retriable(&error) {
                    eprintln!(
                        "{} attempt {} failed: {}. Retrying in {:?}...",
                        operation_name,
                        attempt + 1,
                        error,
                        config.delay
                    );
                    continue;
                } else {
                    // Either we've exhausted retries or the error is not retriable
                    if attempt == max_attempts - 1 {
                        eprintln!(
                            "{} failed after {} attempts: {}",
                            operation_name, max_attempts, error
                        );
                    } else {
                        eprintln!(
                            "{} failed with non-retriable error: {}",
                            operation_name, error
                        );
                    }
                    return Err(error);
                }
            }
        }
    }

    // This should never be reached, but just in case
    Err(last_error.unwrap())
}
