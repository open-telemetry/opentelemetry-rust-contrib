use opentelemetry::logs::AnyValue;
use opentelemetry::Key;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub(super) trait IntoJson {
    fn as_json_value(&self) -> Value;
}

impl IntoJson for AnyValue {
    fn as_json_value(&self) -> Value {
        serialize_anyvalue(self, 0)
    }
}

const ERROR_MSG: &str = "Value truncated as nested lists/maps are not supported.";

fn serialize_anyvalue(value: &AnyValue, depth: usize) -> Value {
    match value {
        AnyValue::Int(value) => json!(value),
        AnyValue::Double(value) => json!(value),
        AnyValue::String(value) => json!(value.as_str()),
        AnyValue::Boolean(value) => json!(value),
        AnyValue::Bytes(_value) => json!("`AnyValue::Bytes` are not supported."),
        AnyValue::ListAny(value) => {
            if depth > 0 {
                // Do not allow nested lists.
                json!(ERROR_MSG)
            } else {
                serialize_anyvalue_slice(value, depth)
            }
        }
        AnyValue::Map(value) => {
            if depth > 0 {
                // Do not allow nested maps.
                json!(ERROR_MSG)
            } else {
                serialize_hashmap_of_anyvalue(value, depth)
            }
        }
        &_ => Value::Null,
    }
}

fn serialize_anyvalue_slice(value: &[AnyValue], depth: usize) -> Value {
    Value::Array(
        value
            .iter()
            .map(|v| serialize_anyvalue(v, depth + 1))
            .collect(),
    )
}

fn serialize_hashmap_of_anyvalue(value: &HashMap<Key, AnyValue>, depth: usize) -> Value {
    Value::Object(
        value
            .iter()
            .map(|(k, v)| (k.to_string(), serialize_anyvalue(v, depth + 1)))
            .collect::<Map<String, Value>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::Key;

    #[test]
    fn test_convert_vec_of_any_value_to_string() {
        let vec = vec![
            AnyValue::Int(1),
            AnyValue::Int(2),
            AnyValue::Int(3),
            AnyValue::Int(0),
            AnyValue::Int(-2),
        ];
        let result = AnyValue::ListAny(Box::new(vec)).as_json_value();
        assert_eq!(result, json!([1, 2, 3, 0, -2]));

        let result = AnyValue::ListAny(Box::default()).as_json_value();
        assert_eq!(result, json!([]));

        let vec_of_bools = vec![AnyValue::Boolean(true), AnyValue::Boolean(false)];
        let result = AnyValue::ListAny(Box::new(vec_of_bools)).as_json_value();
        assert_eq!(result, json!([true, false]));

        let vec_of_doubles = vec![
            AnyValue::Double(1.0),
            AnyValue::Double(-1.0),
            AnyValue::Double(0.0),
            AnyValue::Double(0.1),
            AnyValue::Double(-0.5),
        ];
        let result = AnyValue::ListAny(Box::new(vec_of_doubles)).as_json_value();
        assert_eq!(result, json!([1.0, -1.0, 0.0, 0.1, -0.5]));

        let vec_of_strings = vec![
            AnyValue::String("".into()),
            AnyValue::String("a".into()),
            AnyValue::String(r#"""#.into()),
            AnyValue::String(r#""""#.into()),
            AnyValue::String(r#"foo bar"#.into()),
            AnyValue::String(r#""foo bar""#.into()),
        ];
        let result = AnyValue::ListAny(Box::new(vec_of_strings)).as_json_value();
        assert_eq!(
            result,
            json!(["", "a", "\"", "\"\"", "foo bar", "\"foo bar\""])
        );
    }

    #[test]
    fn test_convert_bytes_panics() {
        let vec = vec![
            AnyValue::Bytes(Box::new(vec![97u8, 98u8, 99u8])),
            AnyValue::Bytes(Box::default()),
        ];
        let result = AnyValue::ListAny(Box::new(vec)).as_json_value();
        assert_eq!(
            result,
            json!([
                "`AnyValue::Bytes` are not supported.",
                "`AnyValue::Bytes` are not supported."
            ])
        );
    }

    #[test]
    fn test_convert_map_of_any_value_to_string() {
        let mut map: HashMap<Key, AnyValue> = HashMap::new();
        map.insert(Key::new("a"), AnyValue::Int(1));
        map.insert(Key::new("b"), AnyValue::Int(2));
        map.insert(Key::new("c"), AnyValue::Int(3));
        map.insert(Key::new("d"), AnyValue::Int(0));
        map.insert(Key::new("e"), AnyValue::Int(-2));
        let result = AnyValue::Map(Box::new(map)).as_json_value();
        assert_eq!(result, json!({"a": 1, "b": 2, "c": 3, "d": 0, "e": -2}));

        let map = HashMap::new();
        let result = AnyValue::Map(Box::new(map)).as_json_value();
        assert_eq!(result, json!({}));

        let mut inner_map = HashMap::new();
        inner_map.insert(Key::new("a"), AnyValue::Int(1));
        inner_map.insert(Key::new("b"), AnyValue::Int(2));
        inner_map.insert(Key::new("c"), AnyValue::Int(3));

        let mut map = HashMap::new();
        map.insert(Key::new("True"), AnyValue::Boolean(true));
        map.insert(Key::new("False"), AnyValue::Boolean(false));
        let result = AnyValue::Map(Box::new(map)).as_json_value();
        assert_eq!(result, json!({"True":true,"False":false}));

        let mut map = HashMap::new();
        map.insert(Key::new("a"), AnyValue::Double(1.0));
        map.insert(Key::new("b"), AnyValue::Double(-1.0));
        map.insert(Key::new("c"), AnyValue::Double(0.0));
        map.insert(Key::new("d"), AnyValue::Double(0.1));
        map.insert(Key::new("e"), AnyValue::Double(-0.5));
        let result = AnyValue::Map(Box::new(map)).as_json_value();
        assert_eq!(result, json!({"a":1.0,"b":-1.0,"c":0.0,"d":0.1,"e":-0.5}));

        let mut map = HashMap::new();
        map.insert(Key::new("a"), AnyValue::String("".into()));
        map.insert(Key::new("b"), AnyValue::String("a".into()));
        map.insert(Key::new("c"), AnyValue::String(r#"""#.into()));
        map.insert(Key::new("d"), AnyValue::String(r#""""#.into()));
        map.insert(Key::new("e"), AnyValue::String(r#"foo bar"#.into()));
        map.insert(Key::new("f"), AnyValue::String(r#""foo bar""#.into()));
        map.insert(Key::new(""), AnyValue::String(r#"empty key"#.into()));
        map.insert(Key::new(r#"""#), AnyValue::String(r#"quote"#.into()));
        map.insert(Key::new(r#""""#), AnyValue::String(r#"quotes"#.into()));
        let result = AnyValue::Map(Box::new(map)).as_json_value();
        assert_eq!(
            result,
            json!({"a":"","b":"a","c":"\"","d":"\"\"","e":"foo bar","f":"\"foo bar\"","":"empty key","\"":"quote","\"\"":"quotes"})
        );
    }

    #[test]
    fn enforce_depth_limit() {
        let complex_vec = vec![
            AnyValue::ListAny(Box::new(vec![AnyValue::Int(3), AnyValue::Int(4)])),
            AnyValue::Int(42),
        ];
        let result = AnyValue::ListAny(Box::new(complex_vec)).as_json_value();
        assert_eq!(
            result.to_string(),
            r#"["Value truncated as nested lists/maps are not supported.",42]"#
        );

        let mut inner_map = HashMap::new();
        inner_map.insert(Key::new("a"), AnyValue::Int(100));

        let mut complex_map = HashMap::new();
        complex_map.insert(Key::new("a"), AnyValue::Int(1));
        complex_map.insert(Key::new("b"), AnyValue::Map(Box::new(inner_map)));
        let result = AnyValue::Map(Box::new(complex_map)).as_json_value();
        assert_eq!(
            result.to_string(),
            r#"{"a":1,"b":"Value truncated as nested lists/maps are not supported."}"#
        );

        // Construct a deeply nested list
        // This will create a structure like: [[[[[[[[[[42]]]]]]]]]]
        let mut current_value = AnyValue::Int(42);
        for _ in 0..10 {
            let vec = vec![current_value];
            current_value = AnyValue::ListAny(Box::new(vec));
        }

        let result = current_value.as_json_value();
        assert_eq!(
            result.to_string(),
            r#"["Value truncated as nested lists/maps are not supported."]"#
        );
    }
}
