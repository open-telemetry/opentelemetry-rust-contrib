//! Resource configuration models for OpenTelemetry.

use opentelemetry::KeyValue;
use opentelemetry_sdk::Resource;
use serde::Deserialize;

use crate::ProviderError;

/// Configuration for Resource attributes and metadata.
#[derive(Deserialize, Debug, Default, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResourceConfig {
    /// List of resource attributes.
    pub attributes: Option<Vec<AttributeConfig>>,

    /// Unsupported experimental detection in this milestone.
    #[serde(rename = "detection/development")]
    pub detection: Option<serde_yaml::Value>,

    /// Unsupported schema_url in this milestone.
    pub schema_url: Option<serde_yaml::Value>,

    /// Unsupported attributes_list in this milestone.
    pub attributes_list: Option<serde_yaml::Value>,
}

/// The declared type of a resource attribute.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum AttributeTypeConfig {
    /// Type was not specified in the configuration (defaults to "string").
    #[default]
    Omitted,
    /// Type was explicitly specified as YAML null (`type: null` or `type: ~`).
    Null,
    /// Type was specified as a string (e.g., "string", "int", etc.).
    String(String),
    /// Type was specified as another YAML type (e.g., integer, boolean).
    Other,
}

impl<'de> Deserialize<'de> for AttributeTypeConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_yaml::Value::deserialize(deserializer)?;
        match value {
            serde_yaml::Value::Null => Ok(AttributeTypeConfig::Null),
            serde_yaml::Value::String(s) => Ok(AttributeTypeConfig::String(s)),
            _ => Ok(AttributeTypeConfig::Other),
        }
    }
}

/// A name-value pair defining a single resource attribute.
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttributeConfig {
    /// The attribute name.
    pub name: String,

    /// The attribute value.
    pub value: serde_yaml::Value,

    /// The declared attribute type (defaults to "string").
    #[serde(default)]
    pub r#type: AttributeTypeConfig,
}

use crate::model::get_mapping_value;

impl ResourceConfig {
    /// Parses the supported resource YAML shape.
    pub(crate) fn from_value(value: &serde_yaml::Value) -> Result<Self, ProviderError> {
        if value.is_null() {
            return Err(ProviderError::InvalidConfiguration(
                "Field 'resource' must be an object, but got null".to_string(),
            ));
        }

        let mapping = value.as_mapping().ok_or_else(|| {
            ProviderError::InvalidConfiguration("Field 'resource' must be an object".to_string())
        })?;

        // Check unsupported fields (even if value is explicit null!)
        if get_mapping_value(mapping, "detection/development").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`resource.detection` is not supported in this milestone".to_string(),
            ));
        }
        if get_mapping_value(mapping, "schema_url").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`resource.schema_url` is not supported in this milestone".to_string(),
            ));
        }
        if get_mapping_value(mapping, "attributes_list").is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`resource.attributes_list` is not supported in this milestone".to_string(),
            ));
        }

        // Check unknown fields
        for (k, _) in mapping {
            if let Some(k_str) = k.as_str() {
                match k_str {
                    "attributes" => {}
                    unknown => {
                        return Err(ProviderError::InvalidConfiguration(format!(
                            "Unknown field 'resource.{}'",
                            unknown
                        )));
                    }
                }
            } else {
                return Err(ProviderError::InvalidConfiguration(
                    "Resource keys must be strings".to_string(),
                ));
            }
        }

        // Validate attributes (if present, must be an array and not null)
        let attributes = match get_mapping_value(mapping, "attributes") {
            Some(val) if val.is_null() => {
                return Err(ProviderError::InvalidConfiguration(
                    "Field 'resource.attributes' must be an array, but got null".to_string(),
                ));
            }
            Some(val) => {
                let seq = val.as_sequence().ok_or_else(|| {
                    ProviderError::InvalidConfiguration(
                        "Field 'resource.attributes' must be an array".to_string(),
                    )
                })?;

                if seq.is_empty() {
                    return Err(ProviderError::InvalidConfiguration(
                        "`resource.attributes` must not be empty if specified".to_string(),
                    ));
                }

                let mut attrs = Vec::with_capacity(seq.len());
                for item in seq {
                    if item.is_null() {
                        return Err(ProviderError::InvalidConfiguration(
                            "Resource attribute must be an object, but got null".to_string(),
                        ));
                    }
                    if !item.is_mapping() {
                        return Err(ProviderError::InvalidConfiguration(
                            "Resource attribute must be an object".to_string(),
                        ));
                    }
                    let attr: AttributeConfig = serde_yaml::from_value(item.clone())
                        .map_err(|e| ProviderError::InvalidConfiguration(e.to_string()))?;
                    attrs.push(attr);
                }
                Some(attrs)
            }
            None => None,
        };

        let config = ResourceConfig {
            attributes,
            detection: None,
            schema_url: None,
            attributes_list: None,
        };
        Ok(config)
    }

    /// Validates the resource configuration.
    pub(crate) fn validate(&self) -> Result<(), ProviderError> {
        if self.detection.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`resource.detection` is not supported in this milestone".to_string(),
            ));
        }
        if self.schema_url.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`resource.schema_url` is not supported in this milestone".to_string(),
            ));
        }
        if self.attributes_list.is_some() {
            return Err(ProviderError::InvalidConfiguration(
                "`resource.attributes_list` is not supported in this milestone".to_string(),
            ));
        }

        if let Some(attributes) = &self.attributes {
            if attributes.is_empty() {
                return Err(ProviderError::InvalidConfiguration(
                    "`resource.attributes` must not be empty if specified".to_string(),
                ));
            }

            for attr in attributes {
                attr.validate()?;
            }
        }

        Ok(())
    }

    /// Converts this validated configuration into an SDK Resource without reading environment variables.
    pub(crate) fn to_resource(&self) -> Result<Resource, ProviderError> {
        let default_service_name: String = std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| format!("unknown_service:{}", name))
            })
            .unwrap_or_else(|| "unknown_service".to_string());

        let mut builder = Resource::builder_empty()
            .with_detector(Box::new(
                opentelemetry_sdk::resource::TelemetryResourceDetector,
            ))
            .with_attribute(KeyValue::new("service.name", default_service_name));

        if let Some(attributes) = &self.attributes {
            for attr in attributes {
                if let Some(kv) = attr.to_key_value()? {
                    builder = builder.with_attribute(kv);
                }
            }
        }
        Ok(builder.build())
    }
}

impl AttributeConfig {
    /// Validates attribute name, declared type, and type/value agreement.
    pub(crate) fn validate(&self) -> Result<(), ProviderError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(ProviderError::InvalidConfiguration(
                "Resource attribute name must not be empty".to_string(),
            ));
        }

        let declared_type = match &self.r#type {
            AttributeTypeConfig::Omitted => "string",
            AttributeTypeConfig::Null => {
                return Err(ProviderError::InvalidConfiguration(format!(
                    "Resource attribute '{}' type cannot be null",
                    self.name
                )));
            }
            AttributeTypeConfig::Other => {
                return Err(ProviderError::InvalidConfiguration(format!(
                    "Resource attribute '{}' type must be a string",
                    self.name
                )));
            }
            AttributeTypeConfig::String(s) => s.as_str(),
        };

        match declared_type {
            "string" | "bool" | "int" | "double" => {}
            "string_array" | "bool_array" | "int_array" | "double_array" => {
                return Err(ProviderError::InvalidConfiguration(format!(
                    "Resource attribute '{}' uses array type '{}' which is not supported in this milestone",
                    self.name, declared_type
                )));
            }
            other => {
                return Err(ProviderError::InvalidConfiguration(format!(
                    "Resource attribute '{}' has unsupported type '{}'",
                    self.name, other
                )));
            }
        }

        // Schema nullBehavior: "the entry is ignored"
        if self.value.is_null() {
            return Ok(());
        }

        match declared_type {
            "string" => {
                if !self.value.is_string() {
                    return Err(ProviderError::InvalidConfiguration(format!(
                        "Resource attribute '{}' declared as type 'string' but value is not a string",
                        self.name
                    )));
                }
            }
            "bool" => {
                if !self.value.is_bool() {
                    return Err(ProviderError::InvalidConfiguration(format!(
                        "Resource attribute '{}' declared as type 'bool' but value is not a boolean",
                        self.name
                    )));
                }
            }
            "int" => {
                // Must be an integer number, not a float or boolean or string
                if self.value.as_i64().is_none() {
                    return Err(ProviderError::InvalidConfiguration(format!(
                        "Resource attribute '{}' declared as type 'int' but value is not an integer",
                        self.name
                    )));
                }
            }
            "double" => {
                if self.value.as_f64().is_none() {
                    return Err(ProviderError::InvalidConfiguration(format!(
                        "Resource attribute '{}' declared as type 'double' but value is not a double",
                        self.name
                    )));
                }
            }
            _ => unreachable!(),
        }

        if self.value.is_sequence() {
            return Err(ProviderError::InvalidConfiguration(format!(
                "Resource attribute '{}' has array value which is not supported in this milestone",
                self.name
            )));
        }

        Ok(())
    }

    /// Converts to a KeyValue attribute, or None if ignored (e.g. null value).
    pub(crate) fn to_key_value(&self) -> Result<Option<KeyValue>, ProviderError> {
        if self.value.is_null() {
            return Ok(None);
        }

        let attr_type = match &self.r#type {
            AttributeTypeConfig::Omitted => "string",
            AttributeTypeConfig::String(s) => s.as_str(),
            AttributeTypeConfig::Null => {
                return Err(ProviderError::InvalidConfiguration(format!(
                    "Resource attribute '{}' type cannot be null",
                    self.name
                )));
            }
            AttributeTypeConfig::Other => {
                return Err(ProviderError::InvalidConfiguration(format!(
                    "Resource attribute '{}' type must be a string",
                    self.name
                )));
            }
        };

        let kv = match attr_type {
            "string" => {
                let s = self.value.as_str().ok_or_else(|| {
                    ProviderError::InvalidConfiguration(format!(
                        "Resource attribute '{}' declared as type 'string' but value is not a string",
                        self.name
                    ))
                })?;
                KeyValue::new(self.name.clone(), s.to_string())
            }
            "bool" => {
                let b = self.value.as_bool().ok_or_else(|| {
                    ProviderError::InvalidConfiguration(format!(
                        "Resource attribute '{}' declared as type 'bool' but value is not a boolean",
                        self.name
                    ))
                })?;
                KeyValue::new(self.name.clone(), b)
            }
            "int" => {
                let i = self.value.as_i64().ok_or_else(|| {
                    ProviderError::InvalidConfiguration(format!(
                        "Resource attribute '{}' declared as type 'int' but value is not an integer",
                        self.name
                    ))
                })?;
                KeyValue::new(self.name.clone(), i)
            }
            "double" => {
                let f = self.value.as_f64().ok_or_else(|| {
                    ProviderError::InvalidConfiguration(format!(
                        "Resource attribute '{}' declared as type 'double' but value is not a double",
                        self.name
                    ))
                })?;
                KeyValue::new(self.name.clone(), f)
            }
            "string_array" | "bool_array" | "int_array" | "double_array" => {
                return Err(ProviderError::InvalidConfiguration(format!(
                    "Resource attribute '{}' uses array type '{}' which is not supported in this milestone",
                    self.name, attr_type
                )));
            }
            other => {
                return Err(ProviderError::InvalidConfiguration(format!(
                    "Resource attribute '{}' has unsupported type '{}'",
                    self.name, other
                )));
            }
        };

        Ok(Some(kv))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::Key;

    #[test]
    fn null_entry_is_ignored_before_a_later_attribute_with_the_same_name() {
        let yaml = r#"
attributes:
  - name: example.key
    value: null
  - name: example.key
    value: configured
"#;
        let config: ResourceConfig = serde_yaml::from_str(yaml).unwrap();
        config.validate().unwrap();
        let resource = config.to_resource().unwrap();
        assert_eq!(
            resource.get(&Key::new("example.key")),
            Some("configured".into())
        );
    }

    #[test]
    fn validate_rejects_unsupported_resource_fields() {
        for (field, expected_message) in [
            (
                "detection/development: {}",
                "`resource.detection` is not supported",
            ),
            (
                "schema_url: https://example.com/schema",
                "`resource.schema_url` is not supported",
            ),
            (
                "attributes_list: key=value",
                "`resource.attributes_list` is not supported",
            ),
        ] {
            let yaml = format!("{field}\n");
            let config: ResourceConfig = serde_yaml::from_str(&yaml).unwrap();
            let err = config.validate().unwrap_err();
            assert!(err.to_string().contains(expected_message));
        }
    }

    #[test]
    fn test_attribute_validation_rejects_empty_name() {
        let yaml = r#"
attributes:
  - name: "   "
    value: "test"
"#;
        let config: ResourceConfig = serde_yaml::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err
            .to_string()
            .contains("Resource attribute name must not be empty"));
    }

    #[test]
    fn test_attribute_validation_rejects_invalid_type_even_with_null_value() {
        let yaml = r#"
attributes:
  - name: example.key
    type: invalid_type
    value: null
"#;
        let config: ResourceConfig = serde_yaml::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err
            .to_string()
            .contains("Resource attribute 'example.key' has unsupported type 'invalid_type'"));
    }

    #[test]
    fn test_attribute_validation_rejects_array_type_even_with_null_value() {
        let yaml = r#"
attributes:
  - name: example.key
    type: string_array
    value: null
"#;
        let config: ResourceConfig = serde_yaml::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err
            .to_string()
            .contains("uses array type 'string_array' which is not supported"));
    }

    #[test]
    fn test_attribute_validation_rejects_explicit_null_type() {
        for yaml in [
            "attributes:\n  - name: k\n    type: null\n    value: 'val'",
            "attributes:\n  - name: k\n    type: null\n    value: null",
            "attributes:\n  - name: k\n    type: ~\n    value: 'val'",
        ] {
            let config: ResourceConfig = serde_yaml::from_str(yaml).unwrap();
            let err = config.validate().unwrap_err();
            assert!(
                err.to_string()
                    .contains("Resource attribute 'k' type cannot be null"),
                "Expected rejection for {yaml}, got: {err}"
            );
        }
    }

    #[test]
    fn test_attribute_validation_rejects_non_string_type() {
        let yaml = r#"
attributes:
  - name: k
    type: 123
    value: "val"
"#;
        let config: ResourceConfig = serde_yaml::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err
            .to_string()
            .contains("Resource attribute 'k' type must be a string"));
    }

    #[test]
    fn test_attribute_validation_rejects_array_values() {
        for yaml in [
            "attributes:\n  - name: k\n    value: ['a', 'b']",
            "attributes:\n  - name: k\n    type: string\n    value: ['a', 'b']",
        ] {
            let config: ResourceConfig = serde_yaml::from_str(yaml).unwrap();
            let err = config.validate().unwrap_err();
            assert!(
                err.to_string()
                    .contains("declared as type 'string' but value is not a string"),
                "Expected scalar type mismatch for {yaml}, got: {err}"
            );
        }
    }

    #[test]
    fn test_attribute_validation_rejects_type_mismatches() {
        for (yaml, expected) in [
            (
                "attributes:\n  - name: k\n    type: string\n    value: 123",
                "declared as type 'string' but value is not a string",
            ),
            (
                "attributes:\n  - name: k\n    type: bool\n    value: 'true'",
                "declared as type 'bool' but value is not a boolean",
            ),
            (
                "attributes:\n  - name: k\n    type: int\n    value: '123'",
                "declared as type 'int' but value is not an integer",
            ),
            (
                "attributes:\n  - name: k\n    type: int\n    value: 3.5",
                "declared as type 'int' but value is not an integer",
            ),
            (
                "attributes:\n  - name: k\n    type: double\n    value: '3.5'",
                "declared as type 'double' but value is not a double",
            ),
        ] {
            let config: ResourceConfig = serde_yaml::from_str(yaml).unwrap();
            let err = config.validate().unwrap_err();
            assert!(
                err.to_string().contains(expected),
                "Expected {expected} for {yaml}, got: {err}"
            );
        }
    }

    #[test]
    fn test_attribute_valid_null_values_are_ignored() {
        let yaml = r#"
attributes:
  - name: string.null
    type: string
    value: null
  - name: bool.null
    type: bool
    value: null
  - name: int.null
    type: int
    value: null
  - name: double.null
    type: double
    value: null
  - name: omitted.null
    value: null
  - name: active.attr
    value: active
"#;
        let config: ResourceConfig = serde_yaml::from_str(yaml).unwrap();
        config.validate().unwrap();
        let res = config.to_resource().unwrap();
        assert_eq!(res.get(&Key::new("active.attr")), Some("active".into()));
        assert_eq!(res.get(&Key::new("string.null")), None);
        assert_eq!(res.get(&Key::new("bool.null")), None);
        assert_eq!(res.get(&Key::new("int.null")), None);
        assert_eq!(res.get(&Key::new("double.null")), None);
        assert_eq!(res.get(&Key::new("omitted.null")), None);
    }

    #[test]
    fn test_attribute_valid_scalars_converted_to_resource() {
        let yaml = r#"
attributes:
  - name: str.key
    type: string
    value: "hello"
  - name: bool.key
    type: bool
    value: true
  - name: int.key
    type: int
    value: 42
  - name: double.key
    type: double
    value: 3.5
  - name: default.key
    value: "default_str"
"#;
        let config: ResourceConfig = serde_yaml::from_str(yaml).unwrap();
        config.validate().unwrap();
        let res = config.to_resource().unwrap();
        assert_eq!(res.get(&Key::new("str.key")), Some("hello".into()));
        assert_eq!(
            res.get(&Key::new("bool.key")),
            Some(opentelemetry::Value::Bool(true))
        );
        assert_eq!(
            res.get(&Key::new("int.key")),
            Some(opentelemetry::Value::I64(42))
        );
        assert_eq!(
            res.get(&Key::new("double.key")),
            Some(opentelemetry::Value::F64(3.5))
        );
        assert_eq!(
            res.get(&Key::new("default.key")),
            Some("default_str".into())
        );
    }
}
