//! Common utilities and validation functions shared across the Geneva uploader crate.

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

        // Test non-printable ASCII
        assert!(validate_user_agent_prefix("AppName").is_err()); // Unit separator
        assert!(validate_user_agent_prefix("AppName").is_err()); // DEL character

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
}
