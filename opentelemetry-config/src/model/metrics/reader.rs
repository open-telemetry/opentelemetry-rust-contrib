//! Metrics Reader Configuration models
//!
//! This module defines the configuration structures and factory traits
//! for Metrics readers used in OpenTelemetry SDKs.

use std::collections::HashMap;

use serde::Deserialize;
use serde_yaml::Value;

/// Metrics reader configuration
#[derive(Debug)]
pub(crate) enum Reader {
    Periodic(Value),
    Pull(Value),
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
                "periodic" => Ok(Reader::Periodic(value)),
                "pull" => Ok(Reader::Pull(value)),
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
                let exporter = periodic["exporter"].clone();
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
                let exporter = pull["exporter"].as_mapping().unwrap();
                assert!(exporter
                    .get(serde_yaml::Value::String("prometheus".to_string()))
                    .is_some());
                let prometheus = exporter
                    .get(serde_yaml::Value::String("prometheus".to_string()))
                    .unwrap();
                if let serde_yaml::Value::Mapping(prometheus_map) = prometheus {
                    assert_eq!(
                        prometheus_map
                            .get(serde_yaml::Value::String("host".to_string()))
                            .unwrap(),
                        "localhost"
                    );
                    assert_eq!(
                        prometheus_map
                            .get(serde_yaml::Value::String("port".to_string()))
                            .unwrap(),
                        9090
                    );
                } else {
                    panic!("Expected Mapping for prometheus");
                }
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
