use std::{borrow::Cow, sync::OnceLock};

use regex::Regex;

use super::{error::ConstraintError, value::StrList};

/// Returns the regex pattern for validating segment/subsegment names.
///
/// Names can contain Unicode letters, numbers, whitespace, and the symbols: `_`, `.`, `:`, `/`, `%`, `&`, `#`, `=`, `+`, `\`, `-`, `@`.
/// Names must be 1-200 characters long.
pub(super) fn name_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"^[\pL\pZ\pN_.:/%&#=+\-@]+$").expect("Invalid name regex"))
}

/// Returns the regex pattern for validating annotation keys.
///
/// Annotation keys must be 1-500 alphanumeric characters or underscores.
pub(super) fn annotation_key_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r"^[a-zA-Z0-9_]{1,500}$").expect("Invalid annotation key regex"))
}

pub(super) fn verify_string_length(s: &str, n: usize) -> Result<(), ConstraintError> {
    if s.len() > n {
        Err(ConstraintError::StringTooLong(n))
    } else {
        Ok(())
    }
}

pub(super) trait MaybeSkip {
    fn skip(&self) -> bool;
}

impl<T: MaybeSkip> MaybeSkip for Option<T> {
    fn skip(&self) -> bool {
        self.is_none() || self.as_ref().is_some_and(|inner| inner.skip())
    }
}

impl MaybeSkip for Cow<'_, str> {
    fn skip(&self) -> bool {
        self.is_empty()
    }
}

impl MaybeSkip for bool {
    fn skip(&self) -> bool {
        !*self
    }
}

impl MaybeSkip for &dyn StrList {
    fn skip(&self) -> bool {
        self.is_empty() || self.into_iter().all(|s| s.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    // Tests for name_regex function

    #[test]
    fn name_regex_valid() {
        let regex = name_regex();

        // Unicode letters
        assert!(regex.is_match("HelloWorld"));
        assert!(regex.is_match("配置名称"));
        assert!(regex.is_match("имя_файла"));

        // Numbers
        assert!(regex.is_match("test123"));
        assert!(regex.is_match("42"));

        // Whitespace
        assert!(regex.is_match("hello world"));
        assert!(regex.is_match("test with spaces"));

        // Allowed symbols: _ . : / % & # = + \ - @
        assert!(regex.is_match("test_name"));
        assert!(regex.is_match("test.name"));
        assert!(regex.is_match("test:name"));
        assert!(regex.is_match("test/name"));
        assert!(regex.is_match("test%name"));
        assert!(regex.is_match("test&name"));
        assert!(regex.is_match("test#name"));
        assert!(regex.is_match("test=name"));
        assert!(regex.is_match("test+name"));
        assert!(regex.is_match("test-name"));
        assert!(regex.is_match("test@name"));

        // Complex combinations
        assert!(regex.is_match("my_service-v1.2.3:prod/api#endpoint"));
        assert!(regex.is_match("user@domain.com"));
    }

    #[test]
    fn name_regex_invalid() {
        let regex = name_regex();

        // Empty string
        assert!(!regex.is_match(""));

        // Prohibited characters
        assert!(!regex.is_match("test<name>"));
        assert!(!regex.is_match("test|name"));
        assert!(!regex.is_match("test;name"));
        assert!(!regex.is_match("test*name"));
        assert!(!regex.is_match("test?name"));
        assert!(!regex.is_match("test[name]"));
        assert!(!regex.is_match("test{name}"));
        assert!(!regex.is_match("test$name"));
        assert!(!regex.is_match("test!name"));
        assert!(!regex.is_match("test^name"));
        assert!(!regex.is_match("test~name"));
        assert!(!regex.is_match("test`name"));

        // Control characters
        assert!(!regex.is_match("test\nname"));
        assert!(!regex.is_match("test\tname"));
        assert!(!regex.is_match("test\x00name"));
    }

    // Tests for annotation_key_regex function

    #[test]
    fn annotation_key_regex_valid() {
        let regex = annotation_key_regex();

        // Simple alphanumeric
        assert!(regex.is_match("key"));
        assert!(regex.is_match("KEY"));
        assert!(regex.is_match("Key123"));

        // With underscores
        assert!(regex.is_match("my_key"));
        assert!(regex.is_match("_private"));
        assert!(regex.is_match("key_name_123"));

        // Single character
        assert!(regex.is_match("a"));
        assert!(regex.is_match("_"));
        assert!(regex.is_match("1"));

        // Maximum length (500 characters)
        let max_length_key = "a".repeat(500);
        assert!(regex.is_match(&max_length_key));
    }

    #[test]
    fn annotation_key_regex_invalid() {
        let regex = annotation_key_regex();

        // Empty string
        assert!(!regex.is_match(""));

        // Other symbols
        assert!(!regex.is_match("key-name"));
        assert!(!regex.is_match("key.name"));
        assert!(!regex.is_match("key:name"));
        assert!(!regex.is_match("key/name"));
        assert!(!regex.is_match("key@name"));
        assert!(!regex.is_match("key#name"));
        assert!(!regex.is_match("key name"));

        // Unicode letters (not allowed)
        assert!(!regex.is_match("配置"));
        assert!(!regex.is_match("имя"));

        // Exceeds maximum length (501 characters)
        let too_long_key = "a".repeat(501);
        assert!(!regex.is_match(&too_long_key));
    }

    // Tests for verify_string_length function

    #[test]
    fn verify_string_length_valid() {
        // Empty string
        assert!(verify_string_length("", 10).is_ok());

        // String within limit
        assert!(verify_string_length("hello", 10).is_ok());
        assert!(verify_string_length("test", 4).is_ok());

        // String exactly at limit
        assert!(verify_string_length("exact", 5).is_ok());

        // Unicode string within limit
        assert!(verify_string_length("配置", 10).is_ok());
    }

    #[test]
    fn verify_string_length_invalid() {
        // String exceeds limit by 1
        let result = verify_string_length("toolong", 6);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ConstraintError::StringTooLong(6));

        // String significantly exceeds limit
        let result = verify_string_length("this is a very long string", 5);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ConstraintError::StringTooLong(5));

        // Unicode string exceeds limit (byte length matters)
        let unicode_str = "配置名称"; // Each character is 3 bytes in UTF-8
        let result = verify_string_length(unicode_str, 5);
        assert!(result.is_err());
    }

    // Comprehensive test for MaybeSkip trait implementations

    #[test]
    fn maybe_skip_implementations() {
        // Test Option<T> implementation

        // None should skip
        let none_option: Option<Cow<str>> = None;
        assert!(none_option.skip());

        // Some with skippable inner value should skip
        let some_empty: Option<Cow<str>> = Some(Cow::Borrowed(""));
        assert!(some_empty.skip());

        // Some with non-skippable inner value should not skip
        let some_value: Option<Cow<str>> = Some(Cow::Borrowed("value"));
        assert!(!some_value.skip());

        // Nested Option with bool
        let none_bool: Option<bool> = None;
        assert!(none_bool.skip());

        let some_false: Option<bool> = Some(false);
        assert!(some_false.skip());

        let some_true: Option<bool> = Some(true);
        assert!(!some_true.skip());

        // Test Cow<str> implementation

        // Empty borrowed string should skip
        let empty_borrowed: Cow<str> = Cow::Borrowed("");
        assert!(empty_borrowed.skip());

        // Empty owned string should skip
        let empty_owned: Cow<str> = Cow::Owned(String::new());
        assert!(empty_owned.skip());

        // Non-empty borrowed string should not skip
        let non_empty_borrowed: Cow<str> = Cow::Borrowed("content");
        assert!(!non_empty_borrowed.skip());

        // Non-empty owned string should not skip
        let non_empty_owned: Cow<str> = Cow::Owned("content".to_string());
        assert!(!non_empty_owned.skip());

        // Test &dyn StrList implementation

        // Empty list should skip
        let empty_list: Vec<String> = vec![];
        let empty_ref: &dyn StrList = &empty_list;
        assert!(empty_ref.skip());

        // List with only empty strings should skip
        let all_empty_list: Vec<String> = vec!["".to_string(), "".to_string()];
        let all_empty_ref: &dyn StrList = &all_empty_list;
        assert!(all_empty_ref.skip());

        // List with at least one non-empty string should not skip
        let mixed_list: Vec<String> = vec!["".to_string(), "value".to_string()];
        let mixed_ref: &dyn StrList = &mixed_list;
        assert!(!mixed_ref.skip());

        // List with all non-empty strings should not skip
        let non_empty_list: Vec<String> = vec!["value1".to_string(), "value2".to_string()];
        let non_empty_ref: &dyn StrList = &non_empty_list;
        assert!(!non_empty_ref.skip());
    }
}
