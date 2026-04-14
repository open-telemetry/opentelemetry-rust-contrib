use std::{
    borrow::Cow,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

use regex::Regex;

/// Converts a system timestamp to X-Ray's epoch seconds floating-point.
pub(super) fn translate_timestamp(st: SystemTime) -> f64 {
    st.duration_since(UNIX_EPOCH)
        .expect("EPOCH is earlier")
        .as_secs_f64()
}

/// Sanitizes annotation keys to conform to X-Ray's naming requirements.
pub(super) fn sanitize_annotation_key(key: &str) -> Cow<'_, str> {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r"[^a-zA-Z0-9_]").expect("Invalid annotation key regex"))
        .replace_all(key, "_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // Tests for translate_timestamp

    #[test]
    fn translate_timestamp() {
        // Test with known timestamp: 2020-01-01 00:00:00 UTC = 1577836800 seconds since epoch
        let timestamp = UNIX_EPOCH + Duration::from_secs(1577836800);
        let result = super::translate_timestamp(timestamp);
        assert_eq!(
            result, 1577836800.0,
            "Should convert SystemTime to epoch seconds"
        );

        // Test with fractional seconds: 1577836800.5 seconds
        let timestamp_with_nanos = UNIX_EPOCH + Duration::from_nanos(1_577_836_800_500_000_000);
        let result = super::translate_timestamp(timestamp_with_nanos);
        assert_eq!(result, 1577836800.5, "Should preserve fractional seconds");

        // Test with UNIX_EPOCH itself
        let epoch = UNIX_EPOCH;
        let result = super::translate_timestamp(epoch);
        assert_eq!(result, 0.0, "UNIX_EPOCH should convert to 0.0");
    }

    // Tests for sanitize_annotation_key

    #[test]
    fn test_sanitize_annotation_key_valid() {
        // Simple alphanumeric
        let result = sanitize_annotation_key("valid_key");
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, "valid_key");

        // Alphanumeric with underscores
        let result = sanitize_annotation_key("snake_case_name");
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, "snake_case_name");

        // With numbers
        let result = sanitize_annotation_key("key123");
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, "key123");

        // Mixed case
        let result = sanitize_annotation_key("MixedCase_Key_123");
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, "MixedCase_Key_123");

        // Only underscores
        let result = sanitize_annotation_key("___");
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, "___");
    }

    #[test]
    fn test_sanitize_annotation_key_invalid() {
        // Spaces replaced with underscores
        let result = sanitize_annotation_key("key with spaces");
        assert!(matches!(result, Cow::Owned(_)));
        assert_eq!(result, "key_with_spaces");

        // Special characters replaced
        let result = sanitize_annotation_key("key-with-dashes");
        assert!(matches!(result, Cow::Owned(_)));
        assert_eq!(result, "key_with_dashes");

        // Multiple special characters
        let result = sanitize_annotation_key("key.with@special#chars!");
        assert!(matches!(result, Cow::Owned(_)));
        assert_eq!(result, "key_with_special_chars_");

        // Unicode characters replaced (each char becomes one underscore, regardless of byte count)
        let result = sanitize_annotation_key("key_配");
        assert!(matches!(result, Cow::Owned(_)));
        assert_eq!(result, "key__"); // "配" is 1 char, so 1 underscore

        // Empty string (no change but still borrowed)
        let result = sanitize_annotation_key("");
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, "");
    }
}
