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
