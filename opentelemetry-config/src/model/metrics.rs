//! # Metrics Configuration module
//!
//! This module defines the configuration structures for Metrics telemetry
//! used in OpenTelemetry SDKs.

pub mod reader;

use crate::{model::metrics::reader::Reader, ProviderError};
use serde::Deserialize;

/// Configuration for MeterProvider according to v1.2.0 schema.
#[derive(Deserialize, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct MeterProviderConfig {
    /// Readers configuration for MeterProvider (required, nonempty).
    pub readers: Option<Vec<Reader>>,

    /// Unsupported views in this milestone.
    pub views: Option<serde_yaml::Value>,

    /// Unsupported exemplar_filter in this milestone.
    pub exemplar_filter: Option<serde_yaml::Value>,

    /// Unsupported experimental meter configurator in this milestone.
    #[serde(rename = "meter_configurator/development")]
    pub meter_configurator: Option<serde_yaml::Value>,

    /// Unsupported experimental view matching mode in this milestone.
    #[serde(rename = "view_matching_mode/development")]
    pub view_matching_mode: Option<serde_yaml::Value>,
}

use crate::model::get_mapping_value;

impl MeterProviderConfig {
    /// Parses the supported MeterProvider YAML shape.
    pub(crate) fn from_value(value: &serde_yaml::Value) -> Result<Self, ProviderError> {
        if value.is_null() {
            return Err(ProviderError::InvalidConfiguration(
                "Field 'meter_provider' must be an object, but got null".to_string(),
            ));
        }

        let mapping = value.as_mapping().ok_or_else(|| {
            ProviderError::InvalidConfiguration(
                "Field 'meter_provider' must be an object".to_string(),
            )
        })?;

        // Check unsupported fields (even if value is explicit null!)
        if get_mapping_value(mapping, "views").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`meter_provider.views` is not supported in this milestone".to_string(),
            ));
        }
        if get_mapping_value(mapping, "exemplar_filter").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`meter_provider.exemplar_filter` is not supported in this milestone".to_string(),
            ));
        }
        if get_mapping_value(mapping, "meter_configurator/development").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`meter_provider.meter_configurator` is not supported in this milestone"
                    .to_string(),
            ));
        }
        if get_mapping_value(mapping, "view_matching_mode/development").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`meter_provider.view_matching_mode` is not supported in this milestone"
                    .to_string(),
            ));
        }

        // Check unknown fields
        for (k, _) in mapping {
            if let Some(k_str) = k.as_str() {
                match k_str {
                    "readers" => {}
                    unknown => {
                        return Err(ProviderError::InvalidConfiguration(format!(
                            "Unknown field 'meter_provider.{}'",
                            unknown
                        )));
                    }
                }
            } else {
                return Err(ProviderError::InvalidConfiguration(
                    "MeterProvider keys must be strings".to_string(),
                ));
            }
        }

        // Validate readers
        let readers_val = get_mapping_value(mapping, "readers").ok_or_else(|| {
            ProviderError::InvalidConfiguration(
                "Missing required field `meter_provider.readers`".to_string(),
            )
        })?;

        if readers_val.is_null() {
            return Err(ProviderError::InvalidConfiguration(
                "Field 'meter_provider.readers' must be an array, but got null".to_string(),
            ));
        }

        let seq = readers_val.as_sequence().ok_or_else(|| {
            ProviderError::InvalidConfiguration(
                "Field 'meter_provider.readers' must be an array".to_string(),
            )
        })?;

        if seq.is_empty() {
            return Err(ProviderError::InvalidConfiguration(
                "`meter_provider.readers` must contain at least one reader".to_string(),
            ));
        }

        let mut readers = Vec::with_capacity(seq.len());
        for item in seq {
            let reader = Reader::from_value(item)?;
            readers.push(reader);
        }

        let config = MeterProviderConfig {
            readers: Some(readers),
            views: None,
            exemplar_filter: None,
            meter_configurator: None,
            view_matching_mode: None,
        };
        Ok(config)
    }

    pub(crate) fn validate(&self) -> Result<(), ProviderError> {
        if self.views.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`meter_provider.views` is not supported in this milestone".to_string(),
            ));
        }
        if self.exemplar_filter.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`meter_provider.exemplar_filter` is not supported in this milestone".to_string(),
            ));
        }
        if self.meter_configurator.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`meter_provider.meter_configurator` is not supported in this milestone"
                    .to_string(),
            ));
        }
        if self.view_matching_mode.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`meter_provider.view_matching_mode` is not supported in this milestone"
                    .to_string(),
            ));
        }

        let readers = self.readers.as_ref().ok_or_else(|| {
            ProviderError::InvalidConfiguration(
                "Missing required field `meter_provider.readers`".to_string(),
            )
        })?;

        if readers.is_empty() {
            return Err(ProviderError::InvalidConfiguration(
                "`meter_provider.readers` must contain at least one reader".to_string(),
            ));
        }

        for reader in readers {
            reader.validate()?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::MeterProviderConfig;

    #[test]
    fn validate_rejects_unsupported_meter_provider_fields() {
        for (field, expected_message) in [
            ("views: []", "`meter_provider.views` is not supported"),
            (
                "exemplar_filter: trace_based",
                "`meter_provider.exemplar_filter` is not supported",
            ),
            (
                "meter_configurator/development: {}",
                "`meter_provider.meter_configurator` is not supported",
            ),
            (
                "view_matching_mode/development: strict",
                "`meter_provider.view_matching_mode` is not supported",
            ),
        ] {
            let yaml =
                format!("readers:\n  - periodic:\n      exporter: {{console: {{}}}}\n{field}\n");
            let config: MeterProviderConfig = serde_yaml::from_str(&yaml).unwrap();
            let err = config.validate().unwrap_err();
            assert!(err.to_string().contains(expected_message));
        }
    }
}
