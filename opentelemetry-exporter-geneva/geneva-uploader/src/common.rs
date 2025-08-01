//! Common utilities and validation functions shared across the Geneva uploader crate.

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use thiserror::Error;

/// Common validation errors
#[derive(Debug, Error)]
pub(crate) enum ValidationError {
    #[error("Invalid user agent prefix: {0}")]
    InvalidUserAgentPrefix(String),
}

pub(crate) type Result<T> = std::result::Result<T, ValidationError>;

/// Validates a user agent prefix for HTTP header compliance
///
/// # Arguments
/// * `prefix` - The user agent prefix to validate
///
/// # Returns
/// * `Ok(())` if valid
/// * `Err(ValidationError::InvalidUserAgentPrefix)` if invalid
///
/// # Validation Rules
/// - Must contain only ASCII printable characters (0x20-0x7E)
/// - Must not contain control characters (especially \r, \n, \0)
/// - Must not exceed 200 characters in length
/// - Must not be empty or only whitespace
pub(crate) fn validate_user_agent_prefix(prefix: &str) -> Result<()> {
    if prefix.trim().is_empty() {
        return Err(ValidationError::InvalidUserAgentPrefix(
            "User agent prefix cannot be empty or only whitespace".to_string(),
        ));
    }

    if prefix.len() > 200 {
        return Err(ValidationError::InvalidUserAgentPrefix(format!(
            "User agent prefix too long: {len} characters (max 200)",
            len = prefix.len()
        )));
    }

    // Check for invalid characters
    for (i, ch) in prefix.char_indices() {
        match ch {
            // Control characters that would break HTTP headers
            '\r' | '\n' | '\0' => {
                return Err(ValidationError::InvalidUserAgentPrefix(format!(
                    "Invalid control character at position {i}: {ch:?}"
                )));
            }
            // Non-ASCII or non-printable characters
            ch if !ch.is_ascii() || (ch as u8) < 0x20 || (ch as u8) > 0x7E => {
                return Err(ValidationError::InvalidUserAgentPrefix(format!(
                    "Invalid character at position {i}: {ch:?} (must be ASCII printable)"
                )));
            }
            _ => {} // Valid character
        }
    }

    Ok(())
}

// Builds a standardized User-Agent header for Geneva services
// TODO: Update the user agent format based on whether custom config will come first or later
// Current format:
// - If prefix is None or empty: "GenevaUploader/0.1"
// - If prefix is provided: "{prefix} (GenevaUploader/0.1)"
pub(crate) fn build_user_agent_header(user_agent_prefix: Option<&str>) -> Result<HeaderValue> {
    let prefix = user_agent_prefix.unwrap_or("");

    // Validate the prefix if provided
    if !prefix.is_empty() {
        validate_user_agent_prefix(prefix)?;
    }

    let user_agent = if prefix.is_empty() {
        "GenevaUploader/0.1".to_string()
    } else {
        format!("{prefix} (GenevaUploader/0.1)")
    };

    HeaderValue::from_str(&user_agent).map_err(|e| {
        ValidationError::InvalidUserAgentPrefix(format!("Failed to create User-Agent header: {e}"))
    })
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
    fn test_validate_user_agent_prefix_valid() {
        assert!(validate_user_agent_prefix("MyApp/1.0").is_ok());
        assert!(validate_user_agent_prefix("Production-Service-2.1.0").is_ok());
        assert!(validate_user_agent_prefix("TestApp_v3").is_ok());
        assert!(validate_user_agent_prefix("App-Name.1.2.3").is_ok());
        assert!(validate_user_agent_prefix("Simple123").is_ok());
    }

    #[test]
    fn test_validate_user_agent_prefix_empty() {
        assert!(validate_user_agent_prefix("").is_err());
        assert!(validate_user_agent_prefix("   ").is_err());
        assert!(validate_user_agent_prefix("\t\n").is_err());

        if let Err(e) = validate_user_agent_prefix("") {
            assert!(e.to_string().contains("cannot be empty"));
        }
    }

    #[test]
    fn test_validate_user_agent_prefix_too_long() {
        let long_prefix = "a".repeat(201);
        let result = validate_user_agent_prefix(&long_prefix);
        assert!(result.is_err());

        if let Err(e) = result {
            assert!(e.to_string().contains("too long"));
            assert!(e.to_string().contains("201 characters"));
        }

        // Test exactly at the limit should be OK
        let max_length_prefix = "a".repeat(200);
        assert!(validate_user_agent_prefix(&max_length_prefix).is_ok());
    }

    #[test]
    fn test_validate_user_agent_prefix_invalid_chars() {
        // Test control characters
        assert!(validate_user_agent_prefix("App\nName").is_err());
        assert!(validate_user_agent_prefix("App\rName").is_err());
        assert!(validate_user_agent_prefix("App\0Name").is_err());
        assert!(validate_user_agent_prefix("App\tName").is_err());

        // Test non-ASCII characters
        assert!(validate_user_agent_prefix("App🚀Name").is_err());
        assert!(validate_user_agent_prefix("Appé").is_err());
        assert!(validate_user_agent_prefix("App中文").is_err());

        // Test non-printable ASCII - construct strings with actual control characters
        let unit_separator = format!("App{}Name", '\u{001F}');
        let del_char = format!("App{}Name", '\u{007F}');
        assert!(validate_user_agent_prefix(&unit_separator).is_err()); // Unit separator (0x1F)
        assert!(validate_user_agent_prefix(&del_char).is_err()); // DEL character (0x7F)

        // Verify error messages contain position information
        if let Err(e) = validate_user_agent_prefix("App\nName") {
            assert!(e.to_string().contains("position 3"));
            assert!(e.to_string().contains("control character"));
        }
    }

    #[test]
    fn test_character_validation_edge_cases() {
        // Test ASCII printable range boundaries
        assert!(validate_user_agent_prefix(" ").is_err()); // Space only should be trimmed to empty
        assert!(validate_user_agent_prefix("App Space").is_ok()); // Space in middle is OK
        assert!(validate_user_agent_prefix("~").is_ok()); // Last printable ASCII (0x7E)
        assert!(validate_user_agent_prefix("!").is_ok()); // First printable ASCII after space (0x21)

        // Test that spaces at the beginning and end are allowed (they're ASCII printable)
        assert!(validate_user_agent_prefix("  ValidApp  ").is_ok()); // Leading/trailing spaces are valid ASCII printable chars
                                                                     // But strings that trim to empty should fail
        assert!(validate_user_agent_prefix("  ").is_err()); // Only spaces should fail
    }

    #[test]
    fn test_build_user_agent_header_without_prefix() {
        let header = build_user_agent_header(None).unwrap();
        assert_eq!(header.to_str().unwrap(), "GenevaUploader/0.1");
    }

    #[test]
    fn test_build_user_agent_header_with_empty_prefix() {
        let header = build_user_agent_header(Some("")).unwrap();
        assert_eq!(header.to_str().unwrap(), "GenevaUploader/0.1");
    }

    #[test]
    fn test_build_user_agent_header_with_valid_prefix() {
        let header = build_user_agent_header(Some("MyApp/2.1.0")).unwrap();
        assert_eq!(header.to_str().unwrap(), "MyApp/2.1.0 (GenevaUploader/0.1)");
    }

    #[test]
    fn test_build_user_agent_header_with_invalid_prefix() {
        let result = build_user_agent_header(Some("Invalid\nPrefix"));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid user agent prefix"));
    }

    #[test]
    fn test_build_geneva_headers_complete() {
        let headers = build_geneva_headers(Some("TestApp/1.0")).unwrap();

        let user_agent = headers.get(USER_AGENT).unwrap();
        assert_eq!(
            user_agent.to_str().unwrap(),
            "TestApp/1.0 (GenevaUploader/0.1)"
        );

        let accept = headers.get(ACCEPT).unwrap();
        assert_eq!(accept.to_str().unwrap(), "application/json");
    }

    #[test]
    fn test_build_geneva_headers_without_prefix() {
        let headers = build_geneva_headers(None).unwrap();

        let user_agent = headers.get(USER_AGENT).unwrap();
        assert_eq!(user_agent.to_str().unwrap(), "GenevaUploader/0.1");

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
            .contains("Invalid user agent prefix"));
    }
}
