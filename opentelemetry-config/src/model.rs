//! # Telemetry Configuration models
//!
//! This module defines the configuration structures for telemetry
//! used in OpenTelemetry SDKs according to the v1.2.0 declarative configuration schema.

pub mod metrics;
pub mod resource;

use serde::Deserialize;

use crate::{
    model::{metrics::MeterProviderConfig, resource::ResourceConfig},
    ProviderError,
};

/// Root configuration for Telemetry according to v1.2.0 schema.
#[derive(Deserialize, Debug, Default, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct Telemetry {
    /// Schema file format version (must be "1.2" for this milestone).
    pub file_format: Option<String>,

    /// MeterProvider configuration.
    pub meter_provider: Option<MeterProviderConfig>,

    /// Resource configuration.
    pub resource: Option<ResourceConfig>,

    /// Unsupported tracer_provider in this milestone.
    pub tracer_provider: Option<serde_yaml::Value>,

    /// Unsupported logger_provider in this milestone.
    pub logger_provider: Option<serde_yaml::Value>,

    /// Unsupported propagator in this milestone.
    pub propagator: Option<serde_yaml::Value>,

    /// Unsupported disabled flag in this milestone.
    pub disabled: Option<serde_yaml::Value>,

    /// Unsupported attribute_limits in this milestone.
    pub attribute_limits: Option<serde_yaml::Value>,

    /// Unsupported log_level in this milestone.
    pub log_level: Option<serde_yaml::Value>,

    /// Detect legacy 'metrics' field to return a clear, helpful error.
    pub metrics: Option<serde_yaml::Value>,
}

/// Helper function to retrieve a value from a YAML mapping by string key.
pub(crate) fn get_mapping_value<'a>(
    mapping: &'a serde_yaml::Mapping,
    key: &str,
) -> Option<&'a serde_yaml::Value> {
    mapping.get(key)
}

impl Telemetry {
    /// Parses the supported YAML shape, preserving explicit null and unsupported-field errors.
    pub(crate) fn from_value(value: &serde_yaml::Value) -> Result<Self, ProviderError> {
        if value.is_null() {
            return Err(ProviderError::InvalidConfiguration(
                "Missing required field `file_format`".to_string(),
            ));
        }

        let mapping = value.as_mapping().ok_or_else(|| {
            ProviderError::InvalidConfiguration("Configuration root must be an object".to_string())
        })?;

        // 1. Detect legacy 'metrics' field
        if get_mapping_value(mapping, "metrics").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "Old 'metrics' configuration format is not supported; use 'meter_provider' with 'file_format: \"1.2\"'".to_string(),
            ));
        }

        // 2. Reject unsupported signals even if explicit null was supplied
        if get_mapping_value(mapping, "tracer_provider").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`tracer_provider` is not supported in this milestone".to_string(),
            ));
        }
        if get_mapping_value(mapping, "logger_provider").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`logger_provider` is not supported in this milestone".to_string(),
            ));
        }
        if get_mapping_value(mapping, "propagator").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`propagator` is not supported in this milestone".to_string(),
            ));
        }
        if get_mapping_value(mapping, "disabled").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`disabled` is not supported in this milestone".to_string(),
            ));
        }
        if get_mapping_value(mapping, "attribute_limits").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`attribute_limits` is not supported in this milestone".to_string(),
            ));
        }
        if get_mapping_value(mapping, "log_level").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`log_level` is not supported in this milestone".to_string(),
            ));
        }

        // 3. Reject unknown top-level fields
        for (k, _) in mapping {
            if let Some(k_str) = k.as_str() {
                match k_str {
                    "file_format" | "meter_provider" | "resource" => {}
                    unknown => {
                        return Err(ProviderError::InvalidConfiguration(format!(
                            "Unknown field '{}' in configuration",
                            unknown
                        )));
                    }
                }
            } else {
                return Err(ProviderError::InvalidConfiguration(
                    "Configuration keys must be strings".to_string(),
                ));
            }
        }

        // 4. Validate file_format
        let file_format_val = get_mapping_value(mapping, "file_format").ok_or_else(|| {
            ProviderError::InvalidConfiguration("Missing required field `file_format`".to_string())
        })?;

        if file_format_val.is_null() {
            return Err(ProviderError::InvalidConfiguration(
                "Field 'file_format' must be a string, but got null".to_string(),
            ));
        }

        let file_format = file_format_val.as_str().ok_or_else(|| {
            ProviderError::InvalidConfiguration("Field 'file_format' must be a string".to_string())
        })?;

        if file_format != "1.2" {
            return Err(ProviderError::InvalidConfiguration(format!(
                "Unsupported file format version '{}'; expected '1.2'",
                file_format
            )));
        }

        // 5. Validate resource (if present, must be an object and not null)
        let resource = match get_mapping_value(mapping, "resource") {
            Some(val) if val.is_null() => {
                return Err(ProviderError::InvalidConfiguration(
                    "Field 'resource' must be an object, but got null".to_string(),
                ));
            }
            Some(val) if !val.is_mapping() => {
                return Err(ProviderError::InvalidConfiguration(
                    "Field 'resource' must be an object".to_string(),
                ));
            }
            Some(val) => Some(ResourceConfig::from_value(val)?),
            None => None,
        };

        // 6. Validate meter_provider (if present, must be an object and not null)
        let meter_provider = match get_mapping_value(mapping, "meter_provider") {
            Some(val) if val.is_null() => {
                return Err(ProviderError::InvalidConfiguration(
                    "Field 'meter_provider' must be an object, but got null".to_string(),
                ));
            }
            Some(val) if !val.is_mapping() => {
                return Err(ProviderError::InvalidConfiguration(
                    "Field 'meter_provider' must be an object".to_string(),
                ));
            }
            Some(val) => Some(MeterProviderConfig::from_value(val)?),
            None => None,
        };

        let config = Telemetry {
            file_format: Some(file_format.to_string()),
            meter_provider,
            resource,
            ..Default::default()
        };
        Ok(config)
    }

    pub(crate) fn validate(&self) -> Result<(), ProviderError> {
        let file_format = self.file_format.as_deref().ok_or_else(|| {
            ProviderError::InvalidConfiguration("Missing required field `file_format`".to_string())
        })?;

        if file_format != "1.2" {
            return Err(ProviderError::InvalidConfiguration(format!(
                "Unsupported file format version '{}'; expected '1.2'",
                file_format
            )));
        }

        if self.metrics.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "Old 'metrics' configuration format is not supported; use 'meter_provider' with 'file_format: \"1.2\"'".to_string(),
            ));
        }

        if self.tracer_provider.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`tracer_provider` is not supported in this milestone".to_string(),
            ));
        }

        if self.logger_provider.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`logger_provider` is not supported in this milestone".to_string(),
            ));
        }

        if self.propagator.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`propagator` is not supported in this milestone".to_string(),
            ));
        }

        if self.disabled.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`disabled` is not supported in this milestone".to_string(),
            ));
        }

        if self.attribute_limits.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`attribute_limits` is not supported in this milestone".to_string(),
            ));
        }

        if self.log_level.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`log_level` is not supported in this milestone".to_string(),
            ));
        }

        if let Some(resource) = &self.resource {
            resource.validate()?;
        }

        if let Some(meter_provider) = &self.meter_provider {
            meter_provider.validate()?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_telemetry_valid() {
        let yaml_str = r#"
          file_format: "1.2"
          meter_provider:
            readers:
              - periodic:
                  exporter:
                    console: {}
          resource:
            attributes:
              - name: service.name
                value: "example-service"
              - name: service.version
                value: "1.0.0"
        "#;
        let telemetry: Telemetry = serde_yaml::from_str(yaml_str).unwrap();
        assert_eq!(telemetry.file_format.as_deref(), Some("1.2"));
        assert!(telemetry.validate().is_ok());
    }

    #[test]
    fn test_missing_file_format() {
        let yaml_str = r#"
          meter_provider:
            readers:
              - periodic:
                  exporter:
                    console: {}
        "#;
        let telemetry: Telemetry = serde_yaml::from_str(yaml_str).unwrap();
        let err = telemetry.validate().unwrap_err();
        assert!(err
            .to_string()
            .contains("Missing required field `file_format`"));
    }

    #[test]
    fn test_unsupported_file_format() {
        let yaml_str = r#"
          file_format: "1.0"
          meter_provider:
            readers:
              - periodic:
                  exporter:
                    console: {}
        "#;
        let telemetry: Telemetry = serde_yaml::from_str(yaml_str).unwrap();
        let err = telemetry.validate().unwrap_err();
        assert!(err
            .to_string()
            .contains("Unsupported file format version '1.0'"));
    }

    #[test]
    fn test_old_metrics_shape_rejected() {
        let yaml_str = r#"
          file_format: "1.2"
          metrics:
            readers:
              - periodic:
                  exporter:
                    console: {}
        "#;
        let telemetry: Telemetry = serde_yaml::from_str(yaml_str).unwrap();
        let err = telemetry.validate().unwrap_err();
        assert!(err
            .to_string()
            .contains("Old 'metrics' configuration format is not supported"));
    }

    #[test]
    fn test_empty_readers_rejected() {
        let yaml_str = r#"
          file_format: "1.2"
          meter_provider:
            readers: []
        "#;
        let telemetry: Telemetry = serde_yaml::from_str(yaml_str).unwrap();
        let err = telemetry.validate().unwrap_err();
        assert!(err.to_string().contains("must contain at least one reader"));
    }

    #[test]
    fn test_unsupported_top_level_signals() {
        let yaml_str = r#"
          file_format: "1.2"
          tracer_provider: {}
        "#;
        let telemetry: Telemetry = serde_yaml::from_str(yaml_str).unwrap();
        let err = telemetry.validate().unwrap_err();
        assert!(err
            .to_string()
            .contains("`tracer_provider` is not supported"));
    }

    #[test]
    fn test_validate_rejects_unsupported_top_level_fields() {
        for (field, expected_message) in [
            ("metrics: {}", "Old 'metrics' configuration format"),
            ("logger_provider: {}", "`logger_provider` is not supported"),
            ("propagator: {}", "`propagator` is not supported"),
            ("disabled: true", "`disabled` is not supported"),
            (
                "attribute_limits: {}",
                "`attribute_limits` is not supported",
            ),
            ("log_level: info", "`log_level` is not supported"),
        ] {
            let yaml = format!("file_format: '1.2'\n{field}\n");
            let telemetry: Telemetry = serde_yaml::from_str(&yaml).unwrap();
            let err = telemetry.validate().unwrap_err();
            assert!(err.to_string().contains(expected_message));
        }
    }

    #[test]
    fn test_unknown_top_level_field_rejected() {
        let yaml_str = r#"
          file_format: "1.2"
          unknown_field: 123
        "#;
        let result: Result<Telemetry, _> = serde_yaml::from_str(yaml_str);
        assert!(result.is_err());
    }
}
