use core::fmt;

use serde::Serialize;

use super::error::ConstraintError;

/// Trait for types that can be serialized as a list of strings.
pub(crate) trait StrList: fmt::Debug + Send + Sync {
    fn len(&self) -> usize;
    fn get(&self, i: usize) -> Option<&str>;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl StrList for Vec<String> {
    fn len(&self) -> usize {
        self.len()
    }

    fn get(&self, i: usize) -> Option<&str> {
        self.as_slice().get(i).map(|s| s.as_str())
    }
}

pub(crate) struct StrListIter<'a> {
    list: &'a dyn StrList,
    idx: usize,
}
impl<'a> Iterator for StrListIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        let elem = self.list.get(self.idx);
        if elem.is_some() {
            self.idx += 1;
        }
        elem
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        // Note we are carfull not to let idx grow beyond the len of the list.
        // remains cannot uderflow
        let remains = self.list.len() - self.idx;
        (remains, Some(remains))
    }
}
impl<'a> IntoIterator for &'a dyn StrList {
    type Item = &'a str;

    type IntoIter = StrListIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        StrListIter { list: self, idx: 0 }
    }
}

impl Serialize for dyn StrList + '_ {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_seq(self)
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct Slice<'a, T>(&'a [T]);

/// Slice of primitive values for metadata.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(untagged)]
pub(crate) enum AnySlice<'a> {
    Bool(Slice<'a, bool>),
    Int(Slice<'a, i64>),
    Float(Slice<'a, f64>),
    String(&'a dyn StrList),
}
macro_rules! impl_from_any_slice {
    ($t:ty, $v:ident) => {
        impl<'a> From<&'a [$t]> for AnySlice<'a> {
            fn from(value: &'a [$t]) -> Self {
                Self::$v(Slice(value))
            }
        }
    };
}
impl_from_any_slice!(bool, Bool);
impl_from_any_slice!(i64, Int);
impl_from_any_slice!(f64, Float);

/// Non-indexed metadata value.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(untagged)]
pub(crate) enum AnyValue<'a> {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(&'a str),
    Slice(AnySlice<'a>),
}

impl<'a> From<AnnotationValue<'a>> for AnyValue<'a> {
    fn from(value: AnnotationValue<'a>) -> Self {
        match value {
            AnnotationValue::String(s) => AnyValue::String(s),
            AnnotationValue::Int(i) => AnyValue::Int(i),
            AnnotationValue::Float(f) => AnyValue::Float(f),
            AnnotationValue::Boolean(b) => AnyValue::Bool(b),
        }
    }
}

/// Indexed annotation value for X-Ray filter expressions.
///
/// String values are limited to 1000 characters.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub(crate) enum AnnotationValue<'a> {
    /// &'a str value (limited to 1000 characters)
    String(&'a str),
    /// Numeric value
    Int(i64),
    /// Numeric value
    Float(f64),
    /// Boolean value
    Boolean(bool),
}

/// AWS origin type for segments.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Origin {
    Ec2,
    Ecs,
    EcsEc2,
    EcsFargate,
    Eks,
    Beanstalk,
    AppRunner,
    Lambda,
}
impl Serialize for Origin {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use Origin::*;
        serializer.serialize_str(match self {
            Ec2 => "AWS::EC2::Instance",
            Ecs => "AWS::ECS::Container",
            EcsEc2 => "AWS::ECS::EC2",
            EcsFargate => "AWS::ECS::Fargate",
            Eks => "AWS::EKS::Container",
            Beanstalk => "AWS::ElasticBeanstalk::Environment",
            AppRunner => "AWS::AppRunner::Service",
            Lambda => "AWS::Lambda::Function",
        })
    }
}
impl core::str::FromStr for Origin {
    type Err = ConstraintError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        use Origin::*;
        match s {
            "aws_ec2" => Ok(Ec2),
            "aws_ecs" => Ok(Ecs),
            "aws_eks" => Ok(Eks),
            "aws_elastic_beanstalk" => Ok(Beanstalk),
            "aws_app_runner" => Ok(AppRunner),
            "aws_lambda" => Ok(Lambda),
            _ => Err(ConstraintError::InvalidOrigin(s.to_owned())),
        }
    }
}

/// Namespace for subsegments (aws or remote).
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Namespace {
    Aws,
    Remote,
}

/// A wrapper around a Vec<(K, V)> which implement
/// Serialize like a Map. Mainly used for XRay metadata
/// and annotation collection instead of a real Map type
/// because we don't actually care for fast seeks, we only
/// want:
/// 1. Fast insertions (Vec is difficult to beat)
/// 2. JSON representation as objects
#[derive(Debug)]
pub(super) struct VectorMap<K, V>(Vec<(K, V)>);

impl<K, V> Default for VectorMap<K, V> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<K, V> VectorMap<K, V> {
    pub fn insert(&mut self, k: K, v: V) {
        self.0.push((k, v));
    }

    pub fn reserve(&mut self, additional: usize) {
        self.0.reserve(additional);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl<K: Serialize, V: Serialize> Serialize for VectorMap<K, V> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_map(self.0.iter().map(|(k, v)| (k, v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests for StrListIter iterator behavior

    #[test]
    fn str_list_iter_valid() {
        // Empty iterator
        let empty: Vec<String> = vec![];
        let empty_list: &dyn StrList = &empty;
        let mut iter = empty_list.into_iter();
        assert_eq!(iter.next(), None);
        assert_eq!(iter.size_hint(), (0, Some(0)));

        // Single element iteration
        let single = vec!["one".to_string()];
        let single_list: &dyn StrList = &single;
        let mut iter = single_list.into_iter();
        assert_eq!(iter.size_hint(), (1, Some(1)));
        assert_eq!(iter.next(), Some("one"));
        assert_eq!(iter.size_hint(), (0, Some(0)));
        assert_eq!(iter.next(), None);

        // Multiple elements iteration
        let multi = vec![
            "first".to_string(),
            "second".to_string(),
            "third".to_string(),
        ];
        let multi_list: &dyn StrList = &multi;
        let mut iter = multi_list.into_iter();
        assert_eq!(iter.size_hint(), (3, Some(3)));
        assert_eq!(iter.next(), Some("first"));
        assert_eq!(iter.size_hint(), (2, Some(2)));
        assert_eq!(iter.next(), Some("second"));
        assert_eq!(iter.size_hint(), (1, Some(1)));
        assert_eq!(iter.next(), Some("third"));
        assert_eq!(iter.size_hint(), (0, Some(0)));
        assert_eq!(iter.next(), None);

        // Collect into Vec
        let list = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let str_list: &dyn StrList = &list;
        let collected: Vec<&str> = str_list.into_iter().collect();
        assert_eq!(collected, vec!["a", "b", "c"]);
    }

    #[test]
    fn str_list_iter_size_hint_consistency() {
        let list = vec!["one".to_string(), "two".to_string(), "three".to_string()];
        let str_list: &dyn StrList = &list;
        let mut iter = str_list.into_iter();

        // Size hint should decrease as we iterate
        let (lower, upper) = iter.size_hint();
        assert_eq!(lower, 3);
        assert_eq!(upper, Some(3));

        iter.next();
        let (lower, upper) = iter.size_hint();
        assert_eq!(lower, 2);
        assert_eq!(upper, Some(2));

        iter.next();
        let (lower, upper) = iter.size_hint();
        assert_eq!(lower, 1);
        assert_eq!(upper, Some(1));

        iter.next();
        let (lower, upper) = iter.size_hint();
        assert_eq!(lower, 0);
        assert_eq!(upper, Some(0));
    }

    // Tests for StrList serialization

    #[test]
    fn str_list_serialization() {
        // Empty list serializes to empty array
        let empty: Vec<String> = vec![];
        let empty_list: &dyn StrList = &empty;
        let json = serde_json::to_string(empty_list).unwrap();
        assert_eq!(json, "[]");

        // Single element
        let single = vec!["hello".to_string()];
        let single_list: &dyn StrList = &single;
        let json = serde_json::to_string(single_list).unwrap();
        assert_eq!(json, r#"["hello"]"#);

        // Multiple elements preserve order
        let multi = vec![
            "first".to_string(),
            "second".to_string(),
            "third".to_string(),
        ];
        let multi_list: &dyn StrList = &multi;
        let json = serde_json::to_string(multi_list).unwrap();
        assert_eq!(json, r#"["first","second","third"]"#);

        // Elements with special characters are properly escaped
        let special = vec!["with \"quotes\"".to_string(), "with\nnewline".to_string()];
        let special_list: &dyn StrList = &special;
        let json = serde_json::to_string(special_list).unwrap();
        assert!(json.contains(r#"with \"quotes\""#));
        assert!(json.contains(r#"\n"#));
    }

    // Comprehensive test for VectorMap operations

    #[test]
    fn vector_map_comprehensive() {
        // Default construction creates empty map
        let mut map: VectorMap<String, i32> = VectorMap::default();
        assert!(map.is_empty());
        assert_eq!(map.len(), 0);

        // Insert single element
        map.insert("first".to_string(), 1);
        assert!(!map.is_empty());
        assert_eq!(map.len(), 1);

        // Insert multiple elements
        map.insert("second".to_string(), 2);
        map.insert("third".to_string(), 3);
        assert_eq!(map.len(), 3);

        // Reserve capacity
        map.reserve(10);
        assert_eq!(map.len(), 3); // Length unchanged after reserve

        // Serialization produces map-like JSON with insertion order preserved
        let json = serde_json::to_string(&map).unwrap();
        assert!(json.contains(r#""first":1"#));
        assert!(json.contains(r#""second":2"#));
        assert!(json.contains(r#""third":3"#));

        // Verify it's a JSON object, not an array
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));

        // Verify insertion order is preserved by checking position
        let first_pos = json.find(r#""first""#).unwrap();
        let second_pos = json.find(r#""second""#).unwrap();
        let third_pos = json.find(r#""third""#).unwrap();
        assert!(first_pos < second_pos);
        assert!(second_pos < third_pos);

        // Test with different value types
        let mut metadata_map: VectorMap<&str, &str> = VectorMap::default();
        metadata_map.insert("key1", "value1");
        metadata_map.insert("key2", "value2");
        let json = serde_json::to_string(&metadata_map).unwrap();
        assert_eq!(json, r#"{"key1":"value1","key2":"value2"}"#);

        // Test with complex nested values
        let mut nested_map: VectorMap<String, Vec<i32>> = VectorMap::default();
        nested_map.insert("numbers".to_string(), vec![1, 2, 3]);
        nested_map.insert("more".to_string(), vec![4, 5]);
        let json = serde_json::to_string(&nested_map).unwrap();
        assert!(json.contains(r#""numbers":[1,2,3]"#));
        assert!(json.contains(r#""more":[4,5]"#));

        // Empty map serializes to empty object
        let empty_map: VectorMap<String, i32> = VectorMap::default();
        let json = serde_json::to_string(&empty_map).unwrap();
        assert_eq!(json, "{}");
    }
}
