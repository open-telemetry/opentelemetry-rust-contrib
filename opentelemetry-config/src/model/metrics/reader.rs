//! Metrics Reader Configuration models
//!
//! This module defines the configuration structures and validation
//! for Metrics readers used in OpenTelemetry SDKs according to the v1.2.0 schema.

use crate::ProviderError;

/// Supported metric reader configuration.
#[derive(Debug)]
pub(crate) enum Reader {
    Periodic(Box<PeriodicReaderConfig>),
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
                let config = PeriodicReaderConfig::from_value(val)?;
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
}

/// Configuration for periodic metric reader.
#[derive(Debug)]
pub(crate) struct PeriodicReaderConfig {
    /// Export interval in milliseconds.
    pub interval: Option<serde_yaml::Value>,

    /// Configured exporter (required).
    pub exporter: Option<serde_yaml::Value>,
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
        if mapping.get("timeout").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "Configuring periodic reader timeout is not supported: the synchronous metric reader in OpenTelemetry SDK 0.33 does not enforce timeouts".to_string(),
            ));
        }

        // 2. Reject unsupported fields (even if explicit null was supplied)
        if mapping.get("max_export_batch_size/development").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`periodic.max_export_batch_size` is not supported in this milestone".to_string(),
            ));
        }
        if mapping.get("producers").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`periodic.producers` is not supported in this milestone".to_string(),
            ));
        }
        if mapping.get("cardinality_limits").is_some() {
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
        let interval = match mapping.get("interval") {
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
        let exporter_val = mapping.get("exporter").ok_or_else(|| {
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
            exporter: Some(exporter_val.clone()),
        };

        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ProviderError> {
        // Validate interval if provided
        if let Some(val) = &self.interval {
            match val {
                serde_yaml::Value::Null => {
                    // Allowed by schema v1.2.0: defaults to 60000 ms
                }
                serde_yaml::Value::Number(n) => {
                    if n.as_u64() == Some(0) {
                        return Err(ProviderError::InvalidConfiguration(
                            "`periodic.interval` of 0 is not supported: the synchronous metric reader in OpenTelemetry SDK 0.33 cannot honor a zero interval".to_string(),
                        ));
                    }
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
                                        "`console.default_histogram_aggregation` value 'base2_exponential_bucket_histogram' is not supported in this milestone".to_string(),
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

    /// Returns the validated interval in milliseconds, using 60,000 for omitted or null values.
    pub(crate) fn interval_ms(&self) -> u64 {
        self.interval
            .as_ref()
            .and_then(serde_yaml::Value::as_u64)
            .unwrap_or(60_000)
    }
}

#[cfg(test)]
mod tests {
    use super::{PeriodicReaderConfig, Reader};

    fn parse_reader(yaml: &str) -> Result<Reader, crate::ProviderError> {
        Reader::from_value(&serde_yaml::from_str(yaml).unwrap())
    }

    fn parse_periodic(yaml: &str) -> Result<PeriodicReaderConfig, crate::ProviderError> {
        PeriodicReaderConfig::from_value(&serde_yaml::from_str(yaml).unwrap())
    }

    #[test]
    fn reader_parser_rejects_invalid_variants() {
        for yaml in ["{}", "periodic: {}\npull: {}", "unknown: {}"] {
            let result = parse_reader(yaml);
            assert!(result.is_err(), "expected {yaml:?} to be rejected");
        }
    }

    #[test]
    fn reader_validation_rejects_pull_readers() {
        assert!(parse_reader("pull: {}")
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
            let err = parse_periodic(&yaml).unwrap_err();
            assert!(err.to_string().contains(expected_message));
        }
    }

    #[test]
    fn periodic_reader_validation_requires_exporter() {
        assert!(parse_periodic("{}")
            .unwrap_err()
            .to_string()
            .contains("Missing required field `exporter`"));
    }
}
