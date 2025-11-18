//! Metrics Reader Configuration models
//!
//! This module defines the configuration structures and factory traits
//! for Metrics readers used in OpenTelemetry SDKs.

use std::collections::HashMap;

use serde::Deserialize;

/// Metrics reader configuration
#[derive(Debug)]
pub enum Reader {
    Periodic(Periodic),
    Pull(Pull),
}

/// Custom deserialization for Reader enum to handle different reader types
impl<'de> Deserialize<'de> for Reader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let map: HashMap<String, serde_yaml::Value> = HashMap::deserialize(deserializer)?;

        if let Some((key, value)) = map.into_iter().next() {
            match key.as_str() {
                "periodic" => {
                    let variant: Periodic =
                        serde_yaml::from_value(value).map_err(serde::de::Error::custom)?;
                    Ok(Reader::Periodic(variant))
                }
                "pull" => {
                    let variant: Pull =
                        serde_yaml::from_value(value).map_err(serde::de::Error::custom)?;
                    Ok(Reader::Pull(variant))
                }
                _ => Err(serde::de::Error::unknown_variant(
                    &key,
                    &["periodic", "pull"],
                )),
            }
        } else {
            Err(serde::de::Error::custom("Empty map"))
        }
    }
}

#[derive(serde::Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Periodic {
    /// Configure delay interval (in milliseconds) between start of two consecutive exports.
    /// Value must be non-negative.
    /// If omitted or null, 60000 is used.
    #[serde(default = "default_interval")]
    pub interval: u64,

    /// Configure maximum allowed time (in milliseconds) to export data.
    /// Value must be non-negative. A value of 0 indicates no limit (infinity).
    /// If omitted or null, 30000 is used.
    #[serde(default = "default_timeout")]
    pub timeout: u64,

    /// Exporter configuration
    pub exporter: serde_yaml::Value,
}

fn default_interval() -> u64 {
    60000
}

fn default_timeout() -> u64 {
    30000
}

#[derive(serde::Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Pull {
    pub exporter: Option<PullExporter>,
}

#[derive(serde::Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct PullExporter {
    pub prometheus: Option<PullExporterPrometheus>,
}

#[derive(serde::Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct PullExporterPrometheus {
    pub host: String,
    pub port: u16,
}

#[derive(serde::Deserialize, Debug)]
#[serde(deny_unknown_fields, rename_all = "lowercase")]
pub enum Temporality {
    Cumulative,
    Delta,
}

#[derive(serde::Deserialize, Debug)]
pub enum Protocol {
    #[serde(rename = "grpc")]
    Grpc,
    #[serde(rename = "http/protobuf")]
    HttpBinary,
    #[serde(rename = "http/json")]
    HttpJson,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_periodic_reader() {
        let yaml_data = r#"
        periodic:
          exporter:
            console:
              temporality: cumulative
        "#;
        let reader: Reader = serde_yaml::from_str(yaml_data).unwrap();
        match reader {
            Reader::Periodic(periodic) => {
                let exporter = periodic.exporter;
                if let serde_yaml::Value::Mapping(exporter_map) = exporter {
                    assert!(exporter_map
                        .get(serde_yaml::Value::String("console".to_string()))
                        .is_some());
                } else {
                    panic!("Expected Mapping for exporter");
                }
            }
            _ => panic!("Expected Periodic reader"),
        }
    }

    #[test]
    fn test_deserialize_pull_reader() {
        let yaml_data = r#"
        pull:
          exporter:
            prometheus:
              host: "localhost"
              port: 9090
        "#;
        let reader: Reader = serde_yaml::from_str(yaml_data).unwrap();
        match reader {
            Reader::Pull(pull) => {
                assert!(pull.exporter.is_some());
                let exporter = pull.exporter.unwrap();
                assert!(exporter.prometheus.is_some());
                let prometheus = exporter.prometheus.unwrap();
                assert_eq!(prometheus.host, "localhost");
                assert_eq!(prometheus.port, 9090);
            }
            _ => panic!("Expected Pull reader"),
        }
    }

    #[test]
    fn test_deserialize_invalid_reader() {
        let yaml_data = r#"
        unknown:
          some_field: value
        "#;
        let result: Result<Reader, _> = serde_yaml::from_str(yaml_data);
        assert!(result.is_err());
    }
}
