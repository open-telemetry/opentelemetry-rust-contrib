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

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::Key;

    #[test]
    fn test_convert_vec_of_any_value_to_string() {
        let vec = [
            AnyValue::Int(1),
            AnyValue::Int(2),
            AnyValue::Int(3),
            AnyValue::Int(0),
            AnyValue::Int(-2),
        ];
        let result = vec.as_json_value();
        assert_eq!(result, json!([1, 2, 3, 0, -2]));

        let result = [].as_json_value();
        assert_eq!(result, json!([]));

        let array = [AnyValue::ListAny(Box::new(vec![
            AnyValue::Int(1),
            AnyValue::Int(2),
            AnyValue::Int(3),
        ]))];
        let result = array.as_json_value();
        assert_eq!(result, json!([[1, 2, 3]]));

        let array = [
            AnyValue::ListAny(Box::new(vec![AnyValue::Int(1), AnyValue::Int(2)])),
            AnyValue::ListAny(Box::new(vec![AnyValue::Int(3), AnyValue::Int(4)])),
        ];
        let result = array.as_json_value();
        assert_eq!(result, json!([[1, 2], [3, 4]]));

        let array = [AnyValue::Boolean(true), AnyValue::Boolean(false)];
        let result = array.as_json_value();
        assert_eq!(result, json!([true, false]));

        let array = [
            AnyValue::Double(1.0),
            AnyValue::Double(-1.0),
            AnyValue::Double(0.0),
            AnyValue::Double(0.1),
            AnyValue::Double(-0.5),
        ];
        let result = array.as_json_value();
        assert_eq!(result, json!([1.0, -1.0, 0.0, 0.1, -0.5]));

        let array = [
            AnyValue::String("".into()),
            AnyValue::String("a".into()),
            AnyValue::String(r#"""#.into()),
            AnyValue::String(r#""""#.into()),
            AnyValue::String(r#"foo bar"#.into()),
            AnyValue::String(r#""foo bar""#.into()),
        ];
        let result = array.as_json_value();
        assert_eq!(
            result,
            json!(["", "a", "\"", "\"\"", "foo bar", "\"foo bar\""])
        );
    }

    #[test]
    #[should_panic]
    fn test_convert_bytes_panics() {
        let array = [
            AnyValue::Bytes(Box::new(vec![97u8, 98u8, 99u8])),
            AnyValue::Bytes(Box::default()),
        ];
        let result = array.as_json_value();
        assert_eq!(result, json!(["abc", ""]));
    }

    #[test]
    fn test_convert_map_of_any_value_to_string() {
        let mut map: HashMap<Key, AnyValue> = HashMap::new();
        map.insert(Key::new("a"), AnyValue::Int(1));
        map.insert(Key::new("b"), AnyValue::Int(2));
        map.insert(Key::new("c"), AnyValue::Int(3));
        map.insert(Key::new("d"), AnyValue::Int(0));
        map.insert(Key::new("e"), AnyValue::Int(-2));
        let result = map.as_json_value();
        assert_eq!(result, json!({"a": 1, "b": 2, "c": 3, "d": 0, "e": -2}));

        let map = HashMap::new();
        let result = map.as_json_value();
        assert_eq!(result, json!({}));

        let mut inner_map = HashMap::new();
        inner_map.insert(Key::new("a"), AnyValue::Int(1));
        inner_map.insert(Key::new("b"), AnyValue::Int(2));
        inner_map.insert(Key::new("c"), AnyValue::Int(3));
        let mut map = HashMap::new();
        map.insert(Key::new("d"), AnyValue::Int(4));
        map.insert(Key::new("e"), AnyValue::Int(5));
        map.insert(Key::new("f"), AnyValue::Map(Box::new(inner_map)));
        let result = map.as_json_value();
        assert_eq!(result, json!({"d":4,"e":5,"f":{"a":1,"b":2,"c":3}}));

        let mut map = HashMap::new();
        map.insert(Key::new("True"), AnyValue::Boolean(true));
        map.insert(Key::new("False"), AnyValue::Boolean(false));
        let result = map.as_json_value();
        assert_eq!(result, json!({"True":true,"False":false}));

        let mut map = HashMap::new();
        map.insert(Key::new("a"), AnyValue::Double(1.0));
        map.insert(Key::new("b"), AnyValue::Double(-1.0));
        map.insert(Key::new("c"), AnyValue::Double(0.0));
        map.insert(Key::new("d"), AnyValue::Double(0.1));
        map.insert(Key::new("e"), AnyValue::Double(-0.5));
        let result = map.as_json_value();
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
        let result = map.as_json_value();
        assert_eq!(
            result,
            json!({"a":"","b":"a","c":"\"","d":"\"\"","e":"foo bar","f":"\"foo bar\"","":"empty key","\"":"quote","\"\"":"quotes"})
        );
    }

    #[test]
    fn test_complex_conversions() {
        let mut simple_map = HashMap::new();
        simple_map.insert(Key::new("a"), AnyValue::Int(1));
        simple_map.insert(Key::new("b"), AnyValue::Int(2));

        let empty_map: HashMap<Key, AnyValue> = HashMap::new();

        let simple_vec = vec![AnyValue::Int(1), AnyValue::Int(2)];

        let empty_vec = vec![];

        let mut complex_map = HashMap::new();
        complex_map.insert(Key::new("a"), AnyValue::Map(Box::new(simple_map.clone())));
        complex_map.insert(Key::new("b"), AnyValue::Map(Box::new(empty_map.clone())));
        complex_map.insert(
            Key::new("c"),
            AnyValue::ListAny(Box::new(simple_vec.clone())),
        );
        complex_map.insert(
            Key::new("d"),
            AnyValue::ListAny(Box::new(empty_vec.clone())),
        );
        let result = complex_map.as_json_value();
        assert_eq!(result, json!({"a":{"a":1,"b":2},"b":{},"c":[1,2],"d":[]}));

        let complex_vec = [
            AnyValue::Map(Box::new(simple_map.clone())),
            AnyValue::Map(Box::new(empty_map.clone())),
            AnyValue::ListAny(Box::new(simple_vec.clone())),
            AnyValue::ListAny(Box::new(empty_vec.clone())),
        ];
        let result = complex_vec.as_json_value();
        assert_eq!(result, json!([{"a":1,"b":2},{},[1,2],[]]));

        let mut nested_complex_map = HashMap::new();
        nested_complex_map.insert(Key::new("a"), AnyValue::Map(Box::new(complex_map.clone())));
        let result = nested_complex_map.as_json_value();
        assert_eq!(
            result,
            json!({"a":{"a":{"a":1,"b":2},"b":{},"c":[1,2],"d":[]}})
        );
    }
}
