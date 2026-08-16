//! Helpers shared by the AWS resource detectors.

use opentelemetry::KeyValue;
#[cfg(feature = "detector-aws-ecs")]
use opentelemetry::{Array, StringValue, Value};

/// Builds a blocking HTTP client with proxying disabled and a global timeout.
#[cfg(feature = "_detector-http")]
pub(super) fn blocking_client(timeout: std::time::Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .proxy(None)
        .timeout_global(Some(timeout))
        .build()
        .into()
}

/// Converts a `Result` into an `Option`, reporting the error via [`log_debug`].
#[cfg(feature = "detector-aws-eks")]
#[cfg_attr(not(feature = "internal-logs"), allow(unused_variables))]
pub(super) fn debug_on_error<T, E: std::error::Error>(
    detector: &'static str,
    result: Result<T, E>,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            #[cfg(feature = "internal-logs")]
            tracing::debug!(%detector, %error, "Detector error");
            None
        }
    }
}

/// Converts a `Result` into an `Option`, reporting the error via [`log_warn`].
#[cfg_attr(not(feature = "internal-logs"), allow(unused_variables))]
pub(super) fn warn_on_error<T, E: std::error::Error>(
    detector: &'static str,
    result: Result<T, E>,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            #[cfg(feature = "internal-logs")]
            tracing::warn!(%detector, %error, "Detector error");
            None
        }
    }
}

/// Builds an attribute from an optional value, dropping it when absent or blank.
pub(super) fn opt_kv(key: &'static str, value: Option<String>) -> Option<KeyValue> {
    value.and_then(non_empty).map(|v| KeyValue::new(key, v))
}

/// [`opt_kv`] for the attributes that semantic conventions type as `string[]`,
/// wrapping the single value the metadata endpoints expose in a one-element array.
#[cfg(feature = "detector-aws-ecs")]
pub(super) fn opt_kv_array(key: &'static str, value: Option<String>) -> Option<KeyValue> {
    value
        .and_then(non_empty)
        .map(|v| KeyValue::new(key, Value::Array(Array::from(vec![StringValue::from(v)]))))
}

/// Trims a string and returns `None` if the result is empty.
pub(super) fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else if trimmed.len() == value.len() {
        Some(value)
    } else {
        Some(trimmed.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests for non_empty

    #[test]
    fn non_empty_some() {
        // Branch (b): no whitespace, returns original unchanged
        let result = non_empty("hello".to_owned());
        assert_eq!(result, Some("hello".to_owned()));

        // Branch (c): surrounding whitespace trimmed, returns trimmed owned string
        let result = non_empty("  hello world  ".to_owned());
        assert_eq!(result, Some("hello world".to_owned()));

        // Branch (c): tabs and newlines trimmed
        let result = non_empty("\t  content\n".to_owned());
        assert_eq!(result, Some("content".to_owned()));
    }

    #[test]
    fn non_empty_none() {
        // Empty string
        assert_eq!(non_empty("".to_owned()), None);

        // Whitespace-only string
        assert_eq!(non_empty("   ".to_owned()), None);

        // Tabs and newlines only
        assert_eq!(non_empty("\t\n\r".to_owned()), None);
    }

    // Tests for opt_kv

    #[test]
    fn opt_kv_some() {
        // Some value with no whitespace
        let kv = opt_kv("my.key", Some("my-value".to_owned()));
        let kv = kv.expect("expected Some(KeyValue)");
        assert_eq!(kv.key.as_str(), "my.key");
        assert_eq!(kv.value, opentelemetry::Value::String("my-value".into()));

        // Some value with surrounding whitespace — trimmed
        let kv = opt_kv("trimmed.key", Some("  trimmed  ".to_owned()));
        let kv = kv.expect("expected Some(KeyValue) after trimming");
        assert_eq!(kv.key.as_str(), "trimmed.key");
        assert_eq!(kv.value, opentelemetry::Value::String("trimmed".into()));
    }

    #[test]
    fn opt_kv_none() {
        // None input
        assert!(opt_kv("k", None).is_none());

        // Some empty string
        assert!(opt_kv("k", Some("".to_owned())).is_none());

        // Some whitespace-only string
        assert!(opt_kv("k", Some("   ".to_owned())).is_none());
    }

    // Tests for opt_kv_array

    #[test]
    fn opt_kv_array_some() {
        let kv = opt_kv_array("array.key", Some("v".to_owned()));
        let kv = kv.expect("expected Some(KeyValue)");
        assert_eq!(kv.key.as_str(), "array.key");

        let expected = Value::Array(Array::from(vec![StringValue::from("v")]));
        assert_eq!(kv.value, expected);
    }

    #[test]
    fn opt_kv_array_none() {
        // None input
        assert!(opt_kv_array("k", None).is_none());

        // Some empty string
        assert!(opt_kv_array("k", Some("".to_owned())).is_none());

        // Some whitespace-only string
        assert!(opt_kv_array("k", Some("  ".to_owned())).is_none());
    }
}
