pub(super) mod segment_document_builder;
pub(super) mod value_builder;

use std::{borrow::Cow, collections::HashMap};

use opentelemetry::{Array, StringValue, Value};

use crate::xray_exporter::types::{AnnotationValue, AnySlice, AnyValue, StrList};

/// Core trait enabling the attribute dispatch system.
///
/// Implementors define a static dispatch table mapping attribute keys to handler functions.
/// The const generic `N` specifies the number of handlers this processor registers.
///
/// Each processor must have a unique `ID` used to reference it in the dispatch table. It
/// is up to the implementor to ensure the ID they choose is indeed unique among other
/// [SpanAttributeProcessor].
///
/// The `HANDLERS` array maps attribute keys to functions that process the attribute value
/// and return `true` if they included it (meaning their value will be in the final Segment)
/// or else `false`.
type AttributeKey = &'static str;
type BuilderHandler<'v, Builder> = fn(&mut Builder, &'v Value) -> bool;
pub(super) trait SpanAttributeProcessor<'v, const N: usize> {
    const HANDLERS: [(AttributeKey, BuilderHandler<'v, Self>); N];
}

/// Identifies which processor type handles an attribute.
///
/// Corresponds to the `SpanAttributeProcessor::ID` constant of a registered processor.
pub(super) type ProcessorId = usize;

/// Index into a processor's `HANDLERS` array.
///
/// Used by the dispatch table to identify which specific handler function to invoke.
pub(super) type HandlerId = usize;

/// Central routing table for the attribute translation pipeline.
#[derive(Debug, Default)]
pub(super) struct DispatchTable(HashMap<&'static str, Vec<(ProcessorId, HandlerId)>>);

impl DispatchTable {
    /// Returns all registered handlers for the given attribute key.
    pub fn dispatch(&self, key: &str) -> &[(ProcessorId, HandlerId)] {
        match self.0.get(key) {
            Some(v) => v.as_slice(),
            None => &[],
        }
    }

    /// Registers a processor's handlers into the dispatch table during initialization.
    ///
    /// Called during `SegmentTranslator::new()` to build the routing table.
    pub fn register<'v, const N: usize, P: SpanAttributeProcessor<'v, N>>(
        &mut self,
        processor_id: ProcessorId,
    ) {
        for (handler_id, key) in P::HANDLERS.iter().map(|a| a.0).enumerate() {
            let element = (processor_id, handler_id);
            self.0
                .entry(key)
                .and_modify(|v| v.push(element))
                .or_insert_with(|| vec![element]);
        }
    }
}

/// Extracts a string from a value with zero-copy semantics when possible.
///
/// Returns `Cow::Borrowed` for string values to avoid allocation, falling back to
/// `Cow::Owned` with `to_string()` conversion for other types.
pub(super) fn get_cow(v: &Value) -> Cow<'_, str> {
    match v {
        Value::String(string_value) => Cow::Borrowed(string_value.as_str()),
        v => Cow::Owned(v.to_string()),
    }
}

/// Extracts a string reference from a value if it is a string type.
///
/// Enables zero-copy string handling by returning a borrowed reference. Returns `None`
/// for non-string types rather than converting.
pub(super) fn get_str(v: &Value) -> Option<&str> {
    match v {
        Value::String(string_value) => Some(string_value.as_str()),
        _ => None,
    }
}

/// Extracts an integer from a value, parsing string representations if necessary.
///
/// Handles both native `i64` values and string values containing parseable integers.
pub(super) fn get_integer(v: &Value) -> Option<i64> {
    match v {
        Value::I64(i) => Some(*i),
        Value::String(i) => i.as_str().trim().parse().ok(),
        _ => None,
    }
}

/// Extracts an boolean from a value.
pub(super) fn get_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

/// Extracts a string array from a value if it is an array of strings.
///
/// Returns a trait object enabling zero-copy access to string array elements. Returns
/// `None` for non-string arrays.
pub(super) fn get_string_vec(v: &Value) -> Option<&dyn StrList> {
    match v {
        Value::Array(Array::String(string_values)) => Some(string_values),
        _ => None,
    }
}

/// Converts a value to an X-Ray annotation value if compatible.
///
/// X-Ray annotations are searchable/indexable fields limited to primitive types (bool, int,
/// float, string). Returns `None` for arrays and other complex types that cannot be annotations.
pub(super) fn get_annotation(v: &Value) -> Option<AnnotationValue> {
    match v {
        Value::Bool(b) => Some(AnnotationValue::Boolean(*b)),
        Value::I64(i) => Some(AnnotationValue::Int(*i)),
        Value::F64(f) => Some(AnnotationValue::Float(*f)),
        Value::String(s) => Some(AnnotationValue::String(s.as_str())),
        _ => None,
    }
}

/// Converts a value to an X-Ray metadata value.
///
/// X-Ray metadata supports both primitives and arrays of primitives, making it more flexible
/// than annotations but not searchable. Enables zero-copy string handling by borrowing from
/// the original value.
pub(super) fn get_any_value(v: &Value) -> Option<AnyValue> {
    Some(match v {
        Value::Bool(b) => AnyValue::Bool(*b),
        Value::I64(i) => AnyValue::Int(*i),
        Value::F64(f) => AnyValue::Float(*f),
        Value::String(s) => AnyValue::String(s.as_str()),
        Value::Array(a) => AnyValue::Slice(match a {
            Array::Bool(items) => AnySlice::from(items.as_slice()),
            Array::I64(items) => AnySlice::from(items.as_slice()),
            Array::F64(items) => AnySlice::from(items.as_slice()),
            Array::String(string_values) => AnySlice::String(string_values),
            _ => return None,
        }),
        _ => return None,
    })
}

/// Implements string list trait for OpenTelemetry string value vectors.
///
/// Enables zero-copy access to string array elements by returning borrowed `&str` references
/// from the underlying `Vec<StringValue>`. Used by `get_string_vec()` and `get_any_value()`
/// to avoid allocating when processing string arrays.
impl StrList for Vec<StringValue> {
    fn len(&self) -> usize {
        self.len()
    }

    fn get(&self, i: usize) -> Option<&str> {
        self.as_slice().get(i).map(|s| s.as_str())
    }
}

/// Semantic convention constants for attribute keys.
/// Re-exports the official opentelemetry_semantic_conventions and
/// adds some AWS-specific values.
mod semconv {
    pub use opentelemetry_semantic_conventions::attribute::*;

    pub const AWS_OPERATION: &str = "aws.operation";
    pub const AWS_SERVICE: &str = "aws.service";
    pub const DB_SERVICE: &str = "db.service";
    pub const AWS_REGION: &str = "aws.region";
    pub const AWS_QUEUE_URL: &str = "aws.queue.url";
    pub const AWS_TABLE_NAME: &str = "aws.table.name";
    pub const TELEMETRY_AUTO_VERSION: &str = "telemetry.auto.version";

    pub const HTTP_STATUS_TEXT: &str = "http.status_text";
    pub const AWS_HTTP_ERROR_MESSAGE: &str = "aws.http.error_message";
    pub const AWS_HTTP_ERROR_EVENT: &str = "aws.http.error.event";

    pub const AWS_HTTP_TRACED: &str = "aws.http.traced";
    pub const CODE_MODULE_NAME: &str = "code.module.name";
}

#[cfg(test)]
mod tests {
    use core::f64::consts::PI;

    use super::*;
    use opentelemetry::{Array, Value};

    // Mock processor for testing DispatchTable
    struct MockProcessor;
    impl<'v> SpanAttributeProcessor<'v, 3> for MockProcessor {
        const HANDLERS: [(&'static str, fn(&mut Self, &'v Value) -> bool); 3] = [
            ("attribute.a", |_, _| true),
            ("attribute.c", |_, _| true),
            ("attribute.b", |_, _| true),
        ];
    }

    struct AnotherMockProcessor;
    impl<'v> SpanAttributeProcessor<'v, 2> for AnotherMockProcessor {
        const HANDLERS: [(&'static str, fn(&mut Self, &'v Value) -> bool); 2] = [
            ("attribute.a", |_, _| true), // Overlaps with MockProcessor
            ("attribute.d", |_, _| true),
        ];
    }

    // Tests for DispatchTable::register

    #[test]
    fn dispatch_table_register_valid() {
        let mut table = DispatchTable::default();

        // Register single processor
        table.register::<3, MockProcessor>(0);
        assert_eq!(table.0.len(), 3);

        // Register second processor with overlapping key
        table.register::<2, AnotherMockProcessor>(1);
        assert_eq!(table.0.len(), 4);
    }

    #[test]
    fn dispatch_table_register_empty() {
        // Empty processor
        struct EmptyProcessor;
        impl<'v> SpanAttributeProcessor<'v, 0> for EmptyProcessor {
            const HANDLERS: [(&'static str, fn(&mut Self, &'v Value) -> bool); 0] = [];
        }

        let mut table = DispatchTable::default();
        table.register::<0, EmptyProcessor>(0);
        assert_eq!(table.0.len(), 0);
    }

    // Tests for DispatchTable::dispatch

    #[test]
    fn dispatch_table_dispatch_valid() {
        let mut table = DispatchTable::default();
        table.register::<3, MockProcessor>(0);
        table.register::<2, AnotherMockProcessor>(1);

        // Key found with single handler
        let handlers = table.dispatch("attribute.b");
        assert_eq!(handlers.len(), 1);
        assert_eq!(handlers[0], (0, 2));

        // Key found with multiple handlers
        let handlers = table.dispatch("attribute.a");
        assert_eq!(handlers.len(), 2);
        assert_eq!(handlers[0], (0, 0));
        assert_eq!(handlers[1], (1, 0));

        // Key found with single handler from second processor
        let handlers = table.dispatch("attribute.d");
        assert_eq!(handlers.len(), 1);
        assert_eq!(handlers[0], (1, 1));
    }

    #[test]
    fn dispatch_table_dispatch_invalid() {
        let mut table = DispatchTable::default();
        table.register::<3, MockProcessor>(0);

        // Key not found returns empty slice
        let handlers = table.dispatch("nonexistent.key");
        assert_eq!(handlers.len(), 0);

        // Partial match doesn't work (exact match required)
        let handlers = table.dispatch("attribute");
        assert_eq!(handlers.len(), 0);

        // Case sensitive
        let handlers = table.dispatch("Attribute.a");
        assert_eq!(handlers.len(), 0);

        // Empty table returns empty slice
        let empty_table = DispatchTable::default();
        let handlers = empty_table.dispatch("any.key");
        assert_eq!(handlers.len(), 0);
    }

    // Tests for get_cow

    #[test]
    fn get_cow_valid() {
        // String value returns Borrowed (zero-copy)
        let special = Value::String("hello\nworld\t!".into());
        let cow = get_cow(&special);
        assert!(matches!(cow, Cow::Borrowed(_)));
        assert_eq!(cow, "hello\nworld\t!");
    }

    #[test]
    fn get_cow_other() {
        // Non-string value returns Owned (formatted)
        let bool_val = Value::Bool(true);
        let cow = get_cow(&bool_val);
        assert!(matches!(cow, Cow::Owned(_)));
        assert_eq!(cow, "true");

        // Integer value
        let int_val = Value::I64(42);
        let cow = get_cow(&int_val);
        assert!(matches!(cow, Cow::Owned(_)));
        assert_eq!(cow, "42");

        // Float value
        let float_val = Value::F64(PI);
        let cow = get_cow(&float_val);
        assert!(matches!(cow, Cow::Owned(_)));
        assert_eq!(cow, "3.141592653589793");

        // Array value
        let array_val = Value::Array(Array::Bool(vec![true, false]));
        let cow = get_cow(&array_val);
        assert!(matches!(cow, Cow::Owned(_)));
        assert_eq!(cow, "[true,false]");
    }

    // Tests for get_str

    #[test]
    fn get_str_valid() {
        // String value returns Some(&str)
        let string_val = Value::String("test".into());
        assert_eq!(get_str(&string_val), Some("test"));
    }

    #[test]
    fn get_str_invalid() {
        // Non-string returns None
        let bool_val = Value::Bool(true);
        assert_eq!(get_str(&bool_val), None);

        let int_val = Value::I64(42);
        assert_eq!(get_str(&int_val), None);

        let float_val = Value::F64(PI);
        assert_eq!(get_str(&float_val), None);

        let array_val = Value::Array(Array::String(vec!["test".into()]));
        assert_eq!(get_str(&array_val), None);
    }

    // Tests for get_integer

    #[test]
    fn get_integer_valid() {
        // I64 value returns Some
        let int_val = Value::I64(42);
        assert_eq!(get_integer(&int_val), Some(42));

        // Negative integer
        let negative = Value::I64(-100);
        assert_eq!(get_integer(&negative), Some(-100));

        // Zero
        let zero = Value::I64(0);
        assert_eq!(get_integer(&zero), Some(0));

        // Parseable string returns Some
        let string_int = Value::String("123".into());
        assert_eq!(get_integer(&string_int), Some(123));

        // Negative string integer
        let negative_str = Value::String("-456".into());
        assert_eq!(get_integer(&negative_str), Some(-456));

        // String with leading/trailing whitespace (parse handles this)
        let whitespace = Value::String("  789  ".into());
        assert_eq!(get_integer(&whitespace), Some(789));
    }

    #[test]
    fn get_integer_invalid() {
        // Non-integer returns None
        let bool_val = Value::Bool(true);
        assert_eq!(get_integer(&bool_val), None);

        let float_val = Value::F64(PI);
        assert_eq!(get_integer(&float_val), None);

        // Non-parseable string returns None
        let invalid_str = Value::String("not a number".into());
        assert_eq!(get_integer(&invalid_str), None);

        // Empty string
        let empty = Value::String("".into());
        assert_eq!(get_integer(&empty), None);

        // Float string
        let float_str = Value::String("3.14".into());
        assert_eq!(get_integer(&float_str), None);

        // Array
        let array_val = Value::Array(Array::I64(vec![1, 2, 3]));
        assert_eq!(get_integer(&array_val), None);
    }

    // Tests for get_bool

    #[test]
    fn get_bool_valid() {
        // Bool value returns Some
        let true_val = Value::Bool(true);
        assert_eq!(get_bool(&true_val), Some(true));

        let false_val = Value::Bool(false);
        assert_eq!(get_bool(&false_val), Some(false));
    }

    #[test]
    fn get_bool_invalid() {
        // Non-bool returns None
        let int_val = Value::I64(1);
        assert_eq!(get_bool(&int_val), None);

        let string_val = Value::String("true".into());
        assert_eq!(get_bool(&string_val), None);

        let float_val = Value::F64(1.0);
        assert_eq!(get_bool(&float_val), None);

        let array_val = Value::Array(Array::Bool(vec![true]));
        assert_eq!(get_bool(&array_val), None);
    }

    // Tests for get_string_vec

    #[test]
    fn get_string_vec_valid() {
        // String array returns Some(StrList)
        let string_array = Value::Array(Array::String(vec![
            "first".into(),
            "second".into(),
            "third".into(),
        ]));
        let result = get_string_vec(&string_array);
        assert!(result.is_some());

        let str_list = result.unwrap();
        assert_eq!(str_list.len(), 3);
        assert_eq!(str_list.get(0), Some("first"));
        assert_eq!(str_list.get(1), Some("second"));
        assert_eq!(str_list.get(2), Some("third"));

        // Empty string array
        let empty_array = Value::Array(Array::String(vec![]));
        let result = get_string_vec(&empty_array);
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 0);

        // Single element
        let single = Value::Array(Array::String(vec!["only".into()]));
        let result = get_string_vec(&single);
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 1);
    }

    #[test]
    fn get_string_vec_invalid() {
        // Non-string array returns None
        let bool_array = Value::Array(Array::Bool(vec![true, false]));
        assert!(get_string_vec(&bool_array).is_none());

        let int_array = Value::Array(Array::I64(vec![1, 2, 3]));
        assert!(get_string_vec(&int_array).is_none());

        let float_array = Value::Array(Array::F64(vec![1.0, 2.0]));
        assert!(get_string_vec(&float_array).is_none());

        // Non-array value returns None
        let string_val = Value::String("not an array".into());
        assert!(get_string_vec(&string_val).is_none());

        let int_val = Value::I64(42);
        assert!(get_string_vec(&int_val).is_none());
    }

    // Tests for get_annotation

    #[test]
    fn get_annotation_valid() {
        // Bool returns Some(AnnotationValue::Boolean)
        let bool_val = Value::Bool(true);
        let result = get_annotation(&bool_val);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), AnnotationValue::Boolean(true)));

        // I64 returns Some(AnnotationValue::Int)
        let int_val = Value::I64(42);
        let result = get_annotation(&int_val);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), AnnotationValue::Int(42)));

        // F64 returns Some(AnnotationValue::Float)
        let float_val = Value::F64(PI);
        let result = get_annotation(&float_val);
        assert!(result.is_some());
        if let Some(AnnotationValue::Float(f)) = result {
            assert!((f - PI).abs() < 0.001);
        } else {
            panic!("Expected Float variant");
        }

        // String returns Some(AnnotationValue::String)
        let string_val = Value::String("test".into());
        let result = get_annotation(&string_val);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), AnnotationValue::String("test")));

        // Empty string
        let empty = Value::String("".into());
        let result = get_annotation(&empty);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), AnnotationValue::String("")));
    }

    #[test]
    fn get_annotation_invalid() {
        // Arrays return None (not annotation-compatible)
        let bool_array = Value::Array(Array::Bool(vec![true]));
        assert!(get_annotation(&bool_array).is_none());

        let int_array = Value::Array(Array::I64(vec![1, 2]));
        assert!(get_annotation(&int_array).is_none());

        let float_array = Value::Array(Array::F64(vec![1.0]));
        assert!(get_annotation(&float_array).is_none());

        let string_array = Value::Array(Array::String(vec!["test".into()]));
        assert!(get_annotation(&string_array).is_none());
    }

    // Comprehensive test for get_any_value covering all value types

    #[test]
    fn get_any_value_comprehensive() {
        // Bool converts to AnyValue::Bool
        let bool_val = Value::Bool(true);
        let result = get_any_value(&bool_val);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), AnyValue::Bool(true)));

        let false_val = Value::Bool(false);
        assert!(matches!(
            get_any_value(&false_val).unwrap(),
            AnyValue::Bool(false)
        ));

        // I64 converts to AnyValue::Int
        let int_val = Value::I64(42);
        let result = get_any_value(&int_val);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), AnyValue::Int(42)));

        let negative_int = Value::I64(-100);
        assert!(matches!(
            get_any_value(&negative_int).unwrap(),
            AnyValue::Int(-100)
        ));

        // F64 converts to AnyValue::Float
        let float_val = Value::F64(PI);
        let result = get_any_value(&float_val);
        assert!(result.is_some());
        if let Some(AnyValue::Float(f)) = result {
            assert!((f - PI).abs() < 0.001);
        } else {
            panic!("Expected Float variant");
        }

        // String converts to AnyValue::String (borrowed)
        let string_val = Value::String("test".into());
        let result = get_any_value(&string_val);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), AnyValue::String("test")));

        let empty_string = Value::String("".into());
        assert!(matches!(
            get_any_value(&empty_string).unwrap(),
            AnyValue::String("")
        ));

        // Bool array converts to AnyValue::Slice(AnySlice::Bool)
        let bool_array = Value::Array(Array::Bool(vec![true, false, true]));
        let result = get_any_value(&bool_array);
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap(),
            AnyValue::Slice(AnySlice::Bool(_))
        ));

        // I64 array converts to AnyValue::Slice(AnySlice::Int)
        let int_array = Value::Array(Array::I64(vec![1, 2, 3]));
        let result = get_any_value(&int_array);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), AnyValue::Slice(AnySlice::Int(_))));

        // F64 array converts to AnyValue::Slice(AnySlice::Float)
        let float_array = Value::Array(Array::F64(vec![1.0, 2.0, 3.0]));
        let result = get_any_value(&float_array);
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap(),
            AnyValue::Slice(AnySlice::Float(_))
        ));

        // String array converts to AnyValue::Slice(AnySlice::String)
        let string_array = Value::Array(Array::String(vec!["first".into(), "second".into()]));
        let result = get_any_value(&string_array);
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap(),
            AnyValue::Slice(AnySlice::String(_))
        ));

        // Empty arrays
        let empty_bool_array = Value::Array(Array::Bool(vec![]));
        assert!(matches!(
            get_any_value(&empty_bool_array).unwrap(),
            AnyValue::Slice(AnySlice::Bool(_))
        ));

        let empty_string_array = Value::Array(Array::String(vec![]));
        assert!(matches!(
            get_any_value(&empty_string_array).unwrap(),
            AnyValue::Slice(AnySlice::String(_))
        ));
    }
}
