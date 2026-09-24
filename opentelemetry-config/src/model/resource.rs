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

/// A name-value pair defining a single resource attribute.
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttributeConfig {
    /// The attribute name.
    pub name: String,

    /// The attribute value.
    pub value: serde_yaml::Value,

    /// The declared attribute type (defaults to "string").
    pub r#type: Option<String>,
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
    /// Validates attribute name and type/value agreement.
    pub(crate) fn validate(&self) -> Result<(), ProviderError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(ProviderError::InvalidConfiguration(
                "Resource attribute name must not be empty".to_string(),
            ));
        }

        // Schema nullBehavior: "the entry is ignored"
        if self.value.is_null() {
            return Ok(());
        }

        let attr_type = self.r#type.as_deref().unwrap_or("string");
        match attr_type {
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

        let attr_type = self.r#type.as_deref().unwrap_or("string");
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
}
