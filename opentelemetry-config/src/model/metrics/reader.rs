//! Metrics Reader Configuration models
//!
//! This module defines the configuration structures and validation
//! for Metrics readers used in OpenTelemetry SDKs according to the v1.2.0 schema.

use crate::ProviderError;

/// Supported metric reader configuration.
#[derive(Debug)]
pub(crate) enum Reader {
    Periodic(serde_yaml::Value),
}

impl Reader {
    /// Constructs and validates a `Reader` from a parsed YAML Value.
    pub(crate) fn from_value(value: &serde_yaml::Value) -> Result<Self, ProviderError> {
        if value.is_null() {
            return Err(ProviderError::InvalidConfiguration(
                "Reader must be an object, but got null".to_string(),
            ));
        }

        let mapping = value.as_mapping().ok_or_else(|| {
            ProviderError::InvalidConfiguration("Reader must be an object".to_string())
        })?;

        if mapping.is_empty() {
            return Err(ProviderError::InvalidConfiguration(
                "Reader must specify exactly one reader type ('periodic' or 'pull'), found 0"
                    .to_string(),
            ));
        }

        if mapping.len() > 1 {
            return Err(ProviderError::InvalidConfiguration(format!(
                "Reader must specify exactly one reader type ('periodic' or 'pull'), found {}",
                mapping.len()
            )));
        }

        let (key_val, val) = mapping.iter().next().unwrap();
        let key_str = key_val.as_str().ok_or_else(|| {
            ProviderError::InvalidConfiguration("Reader key must be a string".to_string())
        })?;

        match key_str {
            "periodic" => {
                if val.is_null() {
                    return Err(ProviderError::InvalidConfiguration(
                        "Field 'periodic' must be an object, but got null".to_string(),
                    ));
                }
                if !val.is_mapping() {
                    return Err(ProviderError::InvalidConfiguration(
                        "Field 'periodic' must be an object".to_string(),
                    ));
                }
                Ok(Reader::Periodic(val.clone()))
            }
            "pull" => Err(ProviderError::InvalidConfiguration(
                "Pull readers are not supported in this milestone".to_string(),
            )),
            unknown => Err(ProviderError::InvalidConfiguration(format!(
                "Unknown reader type '{}'; expected 'periodic' or 'pull'",
                unknown
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reader_parser_rejects_invalid_variants() {
        for yaml in ["{}", "periodic: {}\npull: {}", "unknown: {}", "pull: {}"] {
            let value = serde_yaml::from_str(yaml).unwrap();
            assert!(Reader::from_value(&value).is_err());
        }
    }
}
