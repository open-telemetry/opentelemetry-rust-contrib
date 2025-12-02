//! # Telemetry Configuration models
//!
//! This module defines the configuration structures for telemetry
//! used in OpenTelemetry SDKs.

pub mod metrics;

use std::collections::HashMap;

use serde::Deserialize;
use serde_yaml::Value;

use crate::model::metrics::Metrics;

/// Configuration for Telemetry
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub(crate) struct Telemetry {
    /// Metrics telemetry configuration
    pub metrics: Option<Metrics>,

    /// Resource attributes to be associated with all telemetry data
    #[serde(default)]
    pub resource: HashMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml;

    #[test]
    fn test_deserialize_telemetry() {
        let yaml_str = r#"
          metrics:
            readers:
              - periodic:
                  exporter:
                    console: {}
          resource:
            service.name: "example-service"
            service.version: "1.0.0"
        "#;
        let telemetry: Telemetry = serde_yaml::from_str(yaml_str).unwrap();
        assert!(telemetry.metrics.is_some());
        let resource = telemetry.resource;
        assert_eq!(resource.get("service.name").unwrap(), "example-service");
        assert_eq!(resource.get("service.version").unwrap(), "1.0.0");
    }

    #[test]
    fn test_deserialize_invalid_telemetry() {
        let yaml_str = r#"
          metrics:
            readers_invalid_field:
              - periodic:
                  exporter:
                    console: {}
          resource:
            service.name: "example-service"
            service.version: "1.0.0"
          "#;
        let telemetry_result: Result<Telemetry, _> = serde_yaml::from_str(yaml_str);

        if let Err(e) = telemetry_result {
            assert!(e
                .to_string()
                .contains("unknown field `readers_invalid_field`"));
        } else {
            panic!("Expected error due to invalid field, but got Ok");
        }
    }
}
