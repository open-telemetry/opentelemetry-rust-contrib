//! Metrics Reader Configuration models
//!
//! This module defines the configuration structures and validation
//! for Metrics readers used in OpenTelemetry SDKs according to the v1.2.0 schema.

use crate::ProviderError;
use serde::Deserialize;
use std::collections::HashMap;

/// Metrics reader configuration: exactly one of periodic or pull.
#[derive(Debug, Clone)]
pub(crate) enum Reader {
    Periodic(Box<PeriodicReaderConfig>),
    #[allow(dead_code)]
    Pull(serde_yaml::Value),
}

impl<'de> Deserialize<'de> for Reader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let map: HashMap<String, serde_yaml::Value> = HashMap::deserialize(deserializer)?;

        if map.is_empty() {
            return Err(serde::de::Error::custom(
                "Reader must specify exactly one reader type ('periodic' or 'pull'), found 0",
            ));
        }

        if map.len() > 1 {
            return Err(serde::de::Error::custom(format!(
                "Reader must specify exactly one reader type ('periodic' or 'pull'), found {}",
                map.len()
            )));
        }

        let (key, value) = map.into_iter().next().unwrap();
        match key.as_str() {
            "periodic" => {
                let raw_yaml = serde_yaml::to_string(&value).map_err(serde::de::Error::custom)?;
                let mut config: PeriodicReaderConfig =
                    serde_yaml::from_value(value).map_err(serde::de::Error::custom)?;
                config.raw_yaml = raw_yaml;
                Ok(Reader::Periodic(Box::new(config)))
            }
            "pull" => Ok(Reader::Pull(value)),
            _ => Err(serde::de::Error::custom(format!(
                "Unknown reader type '{}'; expected 'periodic' or 'pull'",
                key
            ))),
        }
    }
}

use crate::model::get_mapping_value;

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
                let raw_yaml = serde_yaml::to_string(val)
                    .map_err(|e| ProviderError::InvalidConfiguration(e.to_string()))?;
                let mut config = PeriodicReaderConfig::from_value(val)?;
                config.raw_yaml = raw_yaml;
                Ok(Reader::Periodic(Box::new(config)))
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

    pub(crate) fn validate(&self) -> Result<(), ProviderError> {
        match self {
            Reader::Periodic(periodic) => periodic.validate(),
            Reader::Pull(_) => Err(ProviderError::InvalidConfiguration(
                "Pull readers are not supported in this milestone".to_string(),
            )),
        }
    }
}

/// Configuration for periodic metric reader.
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct PeriodicReaderConfig {
    /// Export interval in milliseconds.
    pub interval: Option<serde_yaml::Value>,

    /// Timeout is unsupported by the synchronous reader in SDK 0.33.
    pub timeout: Option<serde_yaml::Value>,

    /// Unsupported max_export_batch_size in this milestone.
    #[serde(rename = "max_export_batch_size/development")]
    pub max_export_batch_size: Option<serde_yaml::Value>,

    /// Unsupported producers in this milestone.
    pub producers: Option<serde_yaml::Value>,

    /// Unsupported cardinality_limits in this milestone.
    pub cardinality_limits: Option<serde_yaml::Value>,

    /// Configured exporter (required).
    pub exporter: Option<serde_yaml::Value>,

    /// Raw YAML representation of the periodic reader configuration for custom factory callbacks.
    #[serde(skip)]
    pub raw_yaml: String,
}

impl PeriodicReaderConfig {
    /// Parses the supported periodic reader YAML shape.
    pub(crate) fn from_value(value: &serde_yaml::Value) -> Result<Self, ProviderError> {
        if value.is_null() {
            return Err(ProviderError::InvalidConfiguration(
                "Field 'periodic' must be an object, but got null".to_string(),
            ));
        }

        let mapping = value.as_mapping().ok_or_else(|| {
            ProviderError::InvalidConfiguration("Field 'periodic' must be an object".to_string())
        })?;

        // 1. Explicit timeout must be rejected (even if explicit null was supplied)
        if get_mapping_value(mapping, "timeout").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "Configuring periodic reader timeout is not supported: the synchronous metric reader in OpenTelemetry SDK 0.33 does not enforce timeouts".to_string(),
            ));
        }

        // 2. Reject unsupported fields (even if explicit null was supplied)
        if get_mapping_value(mapping, "max_export_batch_size/development").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`periodic.max_export_batch_size` is not supported in this milestone".to_string(),
            ));
        }
        if get_mapping_value(mapping, "producers").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`periodic.producers` is not supported in this milestone".to_string(),
            ));
        }
        if get_mapping_value(mapping, "cardinality_limits").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`periodic.cardinality_limits` is not supported in this milestone".to_string(),
            ));
        }

        // 3. Reject unknown fields
        for (k, _) in mapping {
            if let Some(k_str) = k.as_str() {
                match k_str {
                    "interval" | "exporter" => {}
                    unknown => {
                        return Err(ProviderError::InvalidConfiguration(format!(
                            "Unknown field 'periodic.{}'",
                            unknown
                        )));
                    }
                }
            } else {
                return Err(ProviderError::InvalidConfiguration(
                    "Periodic configuration keys must be strings".to_string(),
                ));
            }
        }

        // 4. Validate interval (null is allowed by schema v1.2.0 and defaults to 60000)
        let interval = match get_mapping_value(mapping, "interval") {
            Some(val) if val.is_null() => Some(serde_yaml::Value::Null),
            Some(val @ serde_yaml::Value::Number(_)) => Some(val.clone()),
            Some(_) => {
                return Err(ProviderError::InvalidConfiguration(
                    "`periodic.interval` must be a non-negative integer".to_string(),
                ));
            }
            None => None,
        };

        // 5. Validate exporter presence (exporter itself cannot be null)
        let exporter_val = get_mapping_value(mapping, "exporter").ok_or_else(|| {
            ProviderError::InvalidConfiguration(
                "Missing required field `exporter` in periodic reader".to_string(),
            )
        })?;

        if exporter_val.is_null() {
            return Err(ProviderError::InvalidConfiguration(
                "Field 'exporter' must be an object, but got null".to_string(),
            ));
        }

        let config = PeriodicReaderConfig {
            interval,
            timeout: None,
            max_export_batch_size: None,
            producers: None,
            cardinality_limits: None,
            exporter: Some(exporter_val.clone()),
            raw_yaml: String::new(),
        };

        Ok(config)
    }

    pub(crate) fn validate(&self) -> Result<(), ProviderError> {
        // Explicit timeout must be rejected because synchronous reader cannot enforce it
        if self.timeout.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "Configuring periodic reader timeout is not supported: the synchronous metric reader in OpenTelemetry SDK 0.33 does not enforce timeouts".to_string(),
            ));
        }

        if self.max_export_batch_size.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`periodic.max_export_batch_size` is not supported in this milestone".to_string(),
            ));
        }

        if self.producers.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`periodic.producers` is not supported in this milestone".to_string(),
            ));
        }

        if self.cardinality_limits.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`periodic.cardinality_limits` is not supported in this milestone".to_string(),
            ));
        }

        // Validate interval if provided
        if let Some(val) = &self.interval {
            match val {
                serde_yaml::Value::Null => {
                    // Allowed by schema v1.2.0: defaults to 60000 ms
                }
                serde_yaml::Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        if i < 0 {
                            return Err(ProviderError::InvalidConfiguration(
                                "`periodic.interval` must be non-negative".to_string(),
                            ));
                        }
                    } else if n.as_u64().is_none() {
                        return Err(ProviderError::InvalidConfiguration(
                            "`periodic.interval` must be an integer".to_string(),
                        ));
                    }
                }
                _ => {
                    return Err(ProviderError::InvalidConfiguration(
                        "`periodic.interval` must be a non-negative integer".to_string(),
                    ));
                }
            }
        }

        // Validate exporter
        let exporter = self.exporter.as_ref().ok_or_else(|| {
            ProviderError::InvalidConfiguration(
                "Missing required field `exporter` in periodic reader".to_string(),
            )
        })?;

        if exporter.is_null() {
            return Err(ProviderError::InvalidConfiguration(
                "Field 'exporter' must be an object, but got null".to_string(),
            ));
        }

        let exporter_map = exporter.as_mapping().ok_or_else(|| {
            ProviderError::InvalidConfiguration(
                "`exporter` in periodic reader must be an object".to_string(),
            )
        })?;

        if exporter_map.is_empty() {
            return Err(ProviderError::InvalidConfiguration(
                "`exporter` must specify exactly one exporter, found 0".to_string(),
            ));
        }

        if exporter_map.len() > 1 {
            return Err(ProviderError::InvalidConfiguration(format!(
                "`exporter` must specify exactly one exporter, found {}",
                exporter_map.len()
            )));
        }

        let (key_val, val) = exporter_map.iter().next().unwrap();
        let exporter_name = key_val.as_str().ok_or_else(|| {
            ProviderError::InvalidConfiguration("Exporter name must be a string".to_string())
        })?;

        match exporter_name {
            "console" => {
                if val.is_null() {
                    // Allowed by schema v1.2.0: defaults to cumulative temporality and explicit_bucket_histogram
                    return Ok(());
                }
                let console_map = val.as_mapping().ok_or_else(|| {
                    ProviderError::InvalidConfiguration(
                        "Field 'console' must be an object or null".to_string(),
                    )
                })?;
                for (k, v) in console_map {
                    let k_str = k.as_str().ok_or_else(|| {
                        ProviderError::InvalidConfiguration(
                            "Console configuration keys must be strings".to_string(),
                        )
                    })?;
                    match k_str {
                        "temporality_preference" => {
                            if v.is_null() {
                                // Allowed by schema v1.2.0: defaults to cumulative
                                continue;
                            }
                            let temp_str = v.as_str().ok_or_else(|| {
                                ProviderError::InvalidConfiguration(
                                    "`console.temporality_preference` must be a string".to_string(),
                                )
                            })?;
                            match temp_str {
                                "cumulative" | "delta" | "low_memory" => {}
                                _ => {
                                    return Err(ProviderError::InvalidConfiguration(format!(
                                        "Unsupported temporality preference '{}' for console exporter; expected 'cumulative', 'delta', or 'low_memory'",
                                        temp_str
                                    )));
                                }
                            }
                        }
                        "default_histogram_aggregation" => {
                            if v.is_null() {
                                // Allowed by schema v1.2.0: defaults to explicit_bucket_histogram
                                continue;
                            }
                            let agg_str = v.as_str().ok_or_else(|| {
                                ProviderError::InvalidConfiguration(
                                    "`console.default_histogram_aggregation` must be a string"
                                        .to_string(),
                                )
                            })?;
                            match agg_str {
                                "explicit_bucket_histogram" => {}
                                "base2_exponential_bucket_histogram" => {
                                    return Err(ProviderError::InvalidConfiguration(
                                        "`console.default_histogram_aggregation` value 'base2_exponential_bucket_histogram' is not supported in this milestone: OpenTelemetry Rust SDK 0.33 does not support exponential histogram aggregation".to_string(),
                                    ));
                                }
                                _ => {
                                    return Err(ProviderError::InvalidConfiguration(format!(
                                        "`console.default_histogram_aggregation` value '{}' is not supported; only 'explicit_bucket_histogram' is supported",
                                        agg_str
                                    )));
                                }
                            }
                        }
                        unknown => {
                            return Err(ProviderError::InvalidConfiguration(format!(
                                "Unknown field '{}' in console exporter configuration",
                                unknown
                            )));
                        }
                    }
                }
            }
            "otlp_http"
            | "otlp_grpc"
            | "otlp_file/development"
            | "otlp_file"
            | "prometheus/development"
            | "prometheus" => {
                return Err(ProviderError::UnsupportedExporter(format!(
                    "Exporter '{}' is not supported in this milestone",
                    exporter_name
                )));
            }
            _custom => {
                // Finding 3: Require registered custom push exporter values to be an object or null
                if !val.is_mapping() && !val.is_null() {
                    return Err(ProviderError::InvalidConfiguration(format!(
                        "Custom exporter '{}' configuration must be an object or null, but found scalar or array",
                        exporter_name
                    )));
                }
            }
        }

        Ok(())
    }

    /// Returns the configured interval in milliseconds (defaulting to 60,000 if not specified).
    pub(crate) fn interval_ms(&self) -> u64 {
        if let Some(serde_yaml::Value::Number(n)) = &self.interval {
            if let Some(i) = n.as_i64() {
                if i >= 0 {
                    return i as u64;
                }
            } else if let Some(u) = n.as_u64() {
                return u;
            }
        }
        60_000
    }
}

#[cfg(test)]
mod tests {
    use super::{PeriodicReaderConfig, Reader};

    #[test]
    fn reader_deserializer_rejects_invalid_variants() {
        for yaml in ["{}", "periodic: {}\npull: {}", "unknown: {}"] {
            let result = serde_yaml::from_str::<Reader>(yaml);
            assert!(result.is_err(), "expected {yaml:?} to be rejected");
        }
    }

    #[test]
    fn reader_validation_rejects_pull_readers() {
        let reader = serde_yaml::from_str::<Reader>("pull: {}").unwrap();
        assert!(reader
            .validate()
            .unwrap_err()
            .to_string()
            .contains("Pull readers are not supported"));
    }

    #[test]
    fn periodic_reader_validation_rejects_unsupported_options() {
        for (field, expected_message) in [
            ("timeout: 1", "periodic reader timeout is not supported"),
            (
                "max_export_batch_size/development: 1",
                "`periodic.max_export_batch_size` is not supported",
            ),
            ("producers: []", "`periodic.producers` is not supported"),
            (
                "cardinality_limits: {}",
                "`periodic.cardinality_limits` is not supported",
            ),
        ] {
            let yaml = format!("exporter: {{console: {{}}}}\n{field}\n");
            let config: PeriodicReaderConfig = serde_yaml::from_str(&yaml).unwrap();
            let err = config.validate().unwrap_err();
            assert!(err.to_string().contains(expected_message));
        }
    }

    #[test]
    fn periodic_reader_validation_requires_exporter() {
        let config = PeriodicReaderConfig::default();
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("Missing required field `exporter`"));
    }
}
