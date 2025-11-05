//! Common utilities and validation functions shared across the Geneva uploader crate.

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use thiserror::Error;

/// Common validation errors
#[derive(Debug, Error)]
pub(crate) enum ValidationError {
    #[error("Invalid user agent: {0}")]
    InvalidUserAgent(String),
}

pub(crate) type Result<T> = std::result::Result<T, ValidationError>;

// Builds a standardized User-Agent header for Geneva services
// Format:
// - If prefix is None or empty: "RustGenevaClient/0.1"
// - If prefix is provided: "{prefix} (RustGenevaClient/0.1)"
//
// Validation:
// - HeaderValue::from_str() automatically rejects control characters (\r, \n, \0)
// - We additionally verify the header can be represented as ASCII via to_str()
pub(crate) fn build_user_agent_header(user_agent_prefix: Option<&str>) -> Result<HeaderValue> {
    let prefix = user_agent_prefix.unwrap_or("");

    // Basic validation - length and non-empty checks
    if !prefix.is_empty() {
        if prefix.trim().is_empty() {
            return Err(ValidationError::InvalidUserAgent(
                "User agent prefix cannot be only whitespace".to_string(),
            ));
        }
        if prefix.len() > 200 {
            return Err(ValidationError::InvalidUserAgent(format!(
                "User agent prefix too long: {} characters (max 200)",
                prefix.len()
            )));
        }
    }

    // Optimize for the no-prefix case - avoid allocation
    let header_value = if prefix.is_empty() {
        HeaderValue::from_static("RustGenevaClient/0.1")
    } else {
        let user_agent = format!("{prefix} (RustGenevaClient/0.1)");
        let header_value = HeaderValue::from_str(&user_agent).map_err(|e| {
            ValidationError::InvalidUserAgent(format!("Invalid User-Agent header: {e}"))
        })?;

        // Verify the header can be represented as valid ASCII string
        // This rejects non-ASCII characters like emojis, Chinese chars, etc.
        header_value.to_str().map_err(|_| {
            ValidationError::InvalidUserAgent(
                "User-Agent contains non-ASCII characters".to_string(),
            )
        })?;

        header_value
    };

    Ok(header_value)
}

// Builds a complete set of HTTP headers for Geneva services
// Returns HTTP headers including User-Agent and Accept
pub(crate) fn build_geneva_headers(user_agent_prefix: Option<&str>) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();

    let user_agent = build_user_agent_header(user_agent_prefix)?;
    headers.insert(USER_AGENT, user_agent);
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_user_agent_header_without_prefix() {
        let header = build_user_agent_header(None).unwrap();
        assert_eq!(header.to_str().unwrap(), "RustGenevaClient/0.1");
    }

    #[test]
    fn test_build_user_agent_header_with_empty_prefix() {
        let header = build_user_agent_header(Some("")).unwrap();
        assert_eq!(header.to_str().unwrap(), "RustGenevaClient/0.1");
    }

    #[test]
    fn test_build_user_agent_header_with_valid_prefix() {
        let header = build_user_agent_header(Some("MyApp/2.1.0")).unwrap();
        assert_eq!(
            header.to_str().unwrap(),
            "MyApp/2.1.0 (RustGenevaClient/0.1)"
        );
    }

    #[test]
    fn test_build_user_agent_header_with_invalid_control_chars() {
        // Control characters are automatically rejected by HeaderValue::from_str()
        assert!(build_user_agent_header(Some("Invalid\nPrefix")).is_err());
        assert!(build_user_agent_header(Some("App\rName")).is_err());
        assert!(build_user_agent_header(Some("App\0Name")).is_err());
    }

    #[test]
    fn test_build_user_agent_header_with_non_ascii() {
        // Non-ASCII characters should be rejected because we validate with to_str()
        assert!(build_user_agent_header(Some("App€Name")).is_err());
        assert!(build_user_agent_header(Some("App🌏Name")).is_err());
        assert!(build_user_agent_header(Some("App🚀Name")).is_err());
        assert!(build_user_agent_header(Some("App中文Name")).is_err());

        // Verify error message mentions non-ASCII
        let result = build_user_agent_header(Some("App中文"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("non-ASCII"));
    }

    #[test]
    fn test_build_user_agent_header_length_validation() {
        // Test too long prefix
        let long_prefix = "a".repeat(201);
        let result = build_user_agent_header(Some(&long_prefix));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too long"));

        // Test exactly at the limit
        let max_prefix = "a".repeat(200);
        assert!(build_user_agent_header(Some(&max_prefix)).is_ok());
    }

    #[test]
    fn test_build_user_agent_header_whitespace_validation() {
        // Only whitespace should fail
        assert!(build_user_agent_header(Some("   ")).is_err());
        assert!(build_user_agent_header(Some("\t")).is_err());

        // Whitespace within valid text is OK
        assert!(build_user_agent_header(Some("My App")).is_ok());
        assert!(build_user_agent_header(Some("  MyApp  ")).is_ok());
    }

    #[test]
    fn test_build_geneva_headers_complete() {
        let headers = build_geneva_headers(Some("TestApp/1.0")).unwrap();

        let user_agent = headers.get(USER_AGENT).unwrap();
        assert_eq!(
            user_agent.to_str().unwrap(),
            "TestApp/1.0 (RustGenevaClient/0.1)"
        );

        let accept = headers.get(ACCEPT).unwrap();
        assert_eq!(accept.to_str().unwrap(), "application/json");
    }

    #[test]
    fn test_build_geneva_headers_without_prefix() {
        let headers = build_geneva_headers(None).unwrap();

        let user_agent = headers.get(USER_AGENT).unwrap();
        assert_eq!(user_agent.to_str().unwrap(), "RustGenevaClient/0.1");

        let accept = headers.get(ACCEPT).unwrap();
        assert_eq!(accept.to_str().unwrap(), "application/json");
    }

    #[test]
    fn test_build_geneva_headers_with_invalid_prefix() {
        let result = build_geneva_headers(Some("Invalid\rPrefix"));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid User-Agent"));
    }
}
