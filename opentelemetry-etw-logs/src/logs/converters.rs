use opentelemetry::logs::AnyValue;
use opentelemetry::Key;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub(super) trait IntoJson {
    fn as_json_value(&self) -> Value;
}

impl IntoJson for AnyValue {
    fn as_json_value(&self) -> Value {
        match &self {
            AnyValue::Int(value) => json!(value),
            AnyValue::Double(value) => json!(value),
            AnyValue::String(value) => json!(value.to_string()),
            AnyValue::Boolean(value) => json!(value),
            AnyValue::Bytes(_value) => todo!("No support for AnyValue::Bytes yet."),
            AnyValue::ListAny(value) => value.as_json_value(),
            AnyValue::Map(value) => value.as_json_value(),
        }
    }
}

impl IntoJson for HashMap<Key, AnyValue> {
    fn as_json_value(&self) -> Value {
        Value::Object(
            self.iter()
                .map(|(k, v)| (k.to_string(), v.as_json_value()))
                .collect::<Map<String, Value>>(),
        )
    }
}

impl IntoJson for [AnyValue] {
    fn as_json_value(&self) -> Value {
        Value::Array(self.iter().map(IntoJson::as_json_value).collect())
    }
}
