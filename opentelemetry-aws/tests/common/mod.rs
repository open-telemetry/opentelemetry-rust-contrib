//! Common utilities for xray_exporter integration tests.

use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use opentelemetry::{
    trace::{SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState},
    InstrumentationScope, KeyValue, Value,
};
use opentelemetry_aws::xray_exporter::{SegmentDocument, SegmentDocumentExporter};
use opentelemetry_sdk::trace::{SpanData, SpanEvents, SpanLinks};
use serde::Serialize;

use rand::{
    distr::{Distribution, StandardUniform},
    rngs::StdRng,
    Rng, SeedableRng,
};
fn rng_gen<T>() -> T
where
    StandardUniform: Distribution<T>,
{
    thread_local! {
        static RNG : Mutex<StdRng> = Mutex::new(StdRng::seed_from_u64(42));
    }
    RNG.with(|rng| rng.lock().unwrap().random())
}

/// A mock exporter that captures segment documents for testing.
#[derive(Debug, Clone)]
pub struct MockExporter {
    pub documents: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl MockExporter {
    pub fn new() -> Self {
        Self {
            documents: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn get_documents(&self) -> Vec<serde_json::Value> {
        self.documents.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.documents.lock().unwrap().clear()
    }

    pub fn count(&self) -> usize {
        self.documents.lock().unwrap().len()
    }
}
impl Default for MockExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl SegmentDocumentExporter for MockExporter {
    type Error = Infallible;

    async fn export_segment_documents(
        &self,
        batch: Vec<SegmentDocument<'_>>,
    ) -> Result<(), Self::Error> {
        let mut docs = self.documents.lock().unwrap();
        for document in batch {
            // Use serde_json to serialize to Value
            docs.push(serde_json::to_value(&document).unwrap());
        }
        Ok(())
    }
}

/// An iterator over JSON path segments for traversing nested JSON structures.
///
/// This enum provides two ways to specify paths through nested JSON objects:
/// - Dot notation using [`&str`] for regular field names
/// - Slice notation using [`&[&str]`] for field names containing dots
///
/// # Problem
///
/// When traversing JSON structures, field names that contain dots cannot be
/// accessed using simple dot notation because the dot is interpreted as a
/// path separator. For example, a field named `"with.dot"` would be incorrectly
/// split into two separate path segments: `"with"` and `"dot"`.
///
/// # Solution
///
/// Use slice notation to specify the exact field names without splitting:
/// - `"parent.nested.regular"` - splits on dots into `["parent", "nested", "regular"]`
/// - `&["parent", "nested", "with.dot"]` - uses exact field names without splitting
///
/// # Examples
///
/// ```no_run
/// # use serde_json::json;
/// # use common::{get_nested_value, assert_field_eq};
/// // Regular fields work with dot notation
/// let doc = json!({"parent": {"nested": {"regular": "some value"}}});
/// assert_field_eq(&doc, "parent.nested.regular", "some value");
///
/// // Fields with dots require slice notation
/// let doc = json!({"parent": {"nested": {"with.dot": "some value"}}});
/// // This won't work - "with.dot" will be split into "with" and "dot"
/// // assert_field_eq(&doc, "parent.nested.with.dot", "some value");
/// // Use slice notation instead:
/// assert_field_eq(&doc, &["parent", "nested", "with.dot"], "some value");
/// ```
pub enum JsonPath<'a> {
    /// Path created by splitting a string on dots.
    ///
    /// Used when converting from [`&str`] via dot notation like `"parent.child.field"`.
    Split(core::str::Split<'a, char>),

    /// Path created from a slice of exact field names.
    ///
    /// Used when converting from [`&[&str]`] to preserve field names containing dots.
    Slice(core::slice::Iter<'a, &'a str>),
}

impl<'a> Iterator for JsonPath<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            JsonPath::Split(iter) => iter.next(),
            JsonPath::Slice(iter) => iter.next().copied(),
        }
    }
}

/// Converts a string into a [`JsonPath`] by splitting on dots.
///
/// # Examples
///
/// ```no_run
/// # use common::JsonPath;
/// let path: JsonPath = "parent.child.field".into();
/// // Iterates over: "parent", "child", "field"
/// ```
impl<'a> From<&'a str> for JsonPath<'a> {
    fn from(value: &'a str) -> Self {
        Self::Split(value.split('.'))
    }
}

/// Converts a slice of strings into a [`JsonPath`] without splitting.
///
/// This preserves field names that contain dots.
///
/// # Examples
///
/// ```no_run
/// # use common::JsonPath;
/// let path: JsonPath = (&["parent", "child.with.dots"]).into();
/// // Iterates over: "parent", "child.with.dots"
/// ```
impl<'a> From<&'a [&'a str]> for JsonPath<'a> {
    fn from(value: &'a [&'a str]) -> Self {
        Self::Slice(value.iter())
    }
}

/// Retrieves a nested field from a JSON object using a path.
///
/// This function traverses a JSON structure using a [`JsonPath`], returning a
/// reference to the value at that path if it exists. Each segment of the path
/// must correspond to an object key in the JSON structure.
///
/// The path can be specified as:
/// - A string with dot notation: `"parent.child.field"`
/// - A slice for fields with dots: `&["parent", "field.with.dot"]`
///
/// # Examples
///
/// Basic usage with dot notation:
///
/// ```no_run
/// # use serde_json::json;
/// # use common::get_nested_value;
/// let segment = json!({
///     "http": {
///         "request": {
///             "method": "GET",
///             "url": "https://example.com"
///         }
///     }
/// });
///
/// let method = get_nested_value(&segment, "http.request.method");
/// assert_eq!(method, Some(&json!("GET")));
///
/// let missing = get_nested_value(&segment, "http.response.status");
/// assert_eq!(missing, None);
/// ```
///
/// Using slice notation for fields containing dots:
///
/// ```no_run
/// # use serde_json::json;
/// # use common::get_nested_value;
/// let segment = json!({
///     "metadata": {
///         "user.name": "alice"
///     }
/// });
///
/// // Use slice notation to access field with dot in name
/// let name = get_nested_value(&segment, &["metadata", "user.name"]);
/// assert_eq!(name, Some(&json!("alice")));
/// ```
pub fn get_nested_value<'a, 'p, F: Into<JsonPath<'p>>>(
    mut json: &'a serde_json::Value,
    field: F,
) -> Option<&'a serde_json::Value> {
    for part in field.into() {
        json = json.get(part)?;
    }
    Some(json)
}

/// Creates a closure that tests a JSON value for equality.
///
/// This function returns a closure that compares a [`serde_json::Value`] against
/// an expected value. The expected value is serialized to JSON for comparison,
/// allowing any serializable type to be used.
///
/// This is particularly useful with iterator methods like [`Iterator::any`],
/// [`Iterator::find`], or [`Option::is_some_and`] when searching through JSON
/// arrays or testing optional values.
///
/// # Examples
///
/// ```no_run
/// # use serde_json::json;
/// # use common::helper_eq;
/// let metadata = json!({
///     "path": "some/str/path",
///     "timeout": 30,
///     "retries": 3
/// });
///
/// assert!(metadata.get("path").is_some_and(helper_eq("some/str/path")));
/// assert!(metadata.get("timeout").is_some_and(helper_eq(30)));
/// ```
pub fn helper_eq<V: Serialize>(expected: V) -> impl FnOnce(&serde_json::Value) -> bool {
    move |v| *v == serde_json::json!(&expected)
}

/// Asserts that a nested JSON field exists and equals an expected value.
///
/// This function combines [`get_nested_value`] and [`helper_eq`] to verify that
/// a field at the specified path exists and matches the expected value.
/// The assertion fails if the field is missing or has a different value.
///
/// The path can be specified as:
/// - A string with dot notation: `"parent.child.field"`
/// - A slice for fields with dots: `&["parent", "field.with.dot"]`
///
/// # Panics
///
/// Panics if the field does not exist or if its value does not match the expected
/// value. The panic message includes the field path, expected value, and actual value.
///
/// # Examples
///
/// Basic usage with dot notation:
///
/// ```no_run
/// # use serde_json::json;
/// # use common::assert_field_eq;
/// let segment = json!({
///     "name": "DynamoDB.PutItem",
///     "http": {
///         "request": {
///             "method": "POST"
///         }
///     }
/// });
///
/// assert_field_eq(&segment, "name", "DynamoDB.PutItem");
/// assert_field_eq(&segment, "http.request.method", "POST");
/// ```
///
/// Using slice notation for fields containing dots:
///
/// ```no_run
/// # use serde_json::json;
/// # use common::assert_field_eq;
/// let segment = json!({
///     "aws": {
///         "operation.name": "PutItem"
///     }
/// });
///
/// // Use slice notation to access field with dot in name
/// assert_field_eq(&segment, &["aws", "operation.name"], "PutItem");
/// ```
pub fn assert_field_eq<
    'p,
    F: Into<JsonPath<'p>> + core::fmt::Debug + Copy,
    V: Serialize + core::fmt::Debug,
>(
    json: &serde_json::Value,
    field: F,
    expected: V,
) {
    let value = get_nested_value(json, field);
    assert!(
        value.is_some_and(helper_eq(&expected)),
        "Field '{:?}' should exist and equal {:?}, but was {:?}: {}",
        field,
        expected,
        value,
        serde_json::to_string_pretty(json).unwrap()
    );
}

/// Asserts that a nested JSON field exists.
///
/// This function verifies that a field at the specified path exists in the JSON
/// structure, regardless of its value. Use this when you need to verify field
/// presence without checking the specific value.
///
/// The path can be specified as:
/// - A string with dot notation: `"parent.child.field"`
/// - A slice for fields with dots: `&["parent", "field.with.dot"]`
///
/// # Panics
///
/// Panics if the field does not exist at the specified path.
///
/// # Examples
///
/// Basic usage with dot notation:
///
/// ```no_run
/// # use serde_json::json;
/// # use common::assert_field_exists;
/// let segment = json!({
///     "trace_id": "1-67891234-abcdef012345678901234567",
///     "subsegments": []
/// });
///
/// assert_field_exists(&segment, "trace_id");
/// assert_field_exists(&segment, "subsegments");
/// ```
///
/// Using slice notation for fields containing dots:
///
/// ```no_run
/// # use serde_json::json;
/// # use common::assert_field_exists;
/// let segment = json!({
///     "metadata": {
///         "user.id": "12345"
///     }
/// });
///
/// // Use slice notation to check field with dot in name
/// assert_field_exists(&segment, &["metadata", "user.id"]);
/// ```
pub fn assert_field_exists<'p, F: Into<JsonPath<'p>> + core::fmt::Debug + Copy>(
    json: &serde_json::Value,
    field: F,
) {
    assert!(
        get_nested_value(json, field).is_some(),
        "Field '{:?}' should exist: {}",
        field,
        serde_json::to_string_pretty(json).unwrap()
    )
}

/// Asserts that a nested JSON field does not exist.
///
/// This function verifies that a field at the specified path is absent from the
/// JSON structure. Use this to ensure optional fields are correctly omitted when
/// they should not be present.
///
/// The path can be specified as:
/// - A string with dot notation: `"parent.child.field"`
/// - A slice for fields with dots: `&["parent", "field.with.dot"]`
///
/// # Panics
///
/// Panics if the field exists at the specified path.
///
/// # Examples
///
/// Basic usage with dot notation:
///
/// ```no_run
/// # use serde_json::json;
/// # use common::assert_field_not_exists;
/// let segment = json!({
///     "name": "my-operation",
///     "id": "1234567890abcdef"
/// });
///
/// // Verify optional fields are not present
/// assert_field_not_exists(&segment, "error");
/// assert_field_not_exists(&segment, "fault");
/// assert_field_not_exists(&segment, "throttle");
/// ```
///
/// Using slice notation for fields containing dots:
///
/// ```no_run
/// # use serde_json::json;
/// # use common::assert_field_not_exists;
/// let segment = json!({
///     "name": "my-operation"
/// });
///
/// // Verify field with dot in name is absent
/// assert_field_not_exists(&segment, &["metadata", "user.id"]);
/// ```
pub fn assert_field_not_exists<'p, F: Into<JsonPath<'p>> + core::fmt::Debug + Copy>(
    json: &serde_json::Value,
    field: F,
) {
    assert!(
        get_nested_value(json, field).is_none(),
        "Field '{:?}' should not exist: {}",
        field,
        serde_json::to_string_pretty(json).unwrap()
    )
}

/// Creates a valid X-Ray trace ID with the current timestamp.
pub fn create_valid_trace_id() -> TraceId {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as u128;
    let random_part: u128 = rng_gen();
    let trace_id = (timestamp << 96) | (random_part >> 32);
    TraceId::from_bytes(trace_id.to_be_bytes())
}

/// Creates a basic span with minimal attributes.
pub fn create_basic_span(
    name: &'static str,
    kind: SpanKind,
    trace_id: TraceId,
    span_id: SpanId,
    parent_span_id: Option<SpanId>,
) -> SpanData {
    let span_context = SpanContext::new(
        trace_id,
        span_id,
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    );

    let start_time = UNIX_EPOCH + Duration::from_secs(1700000000);
    let end_time = start_time + Duration::from_millis(100);

    SpanData {
        span_context,
        parent_span_id: parent_span_id.unwrap_or(SpanId::INVALID),
        parent_span_is_remote: parent_span_id.is_some(),
        span_kind: kind,
        name: name.into(),
        start_time,
        end_time,
        attributes: vec![],
        dropped_attributes_count: 0,
        events: SpanEvents::default(),
        links: SpanLinks::default(),
        status: Status::Unset,
        instrumentation_scope: InstrumentationScope::builder("test").build(),
    }
}

/// Creates a Lambda handler span (Server span).
pub fn create_lambda_handler_span(
    trace_id: TraceId,
    span_id: SpanId,
    parent_span_id: Option<SpanId>,
) -> SpanData {
    let span_context = SpanContext::new(
        trace_id,
        span_id,
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    );

    let start_time = UNIX_EPOCH + Duration::from_millis(1700000000000);
    let end_time = start_time + Duration::from_millis(130);

    let attributes = vec![
        KeyValue::new("faas.trigger", "http"),
        KeyValue::new(
            "cloud.resource_id",
            "arn:aws:lambda:us-east-1:123456789012:function:my-function",
        ),
        KeyValue::new("faas.invocation_id", "784851f1-0490-42ca-a093-8bbe57fc1e04"),
        KeyValue::new("cloud.account.id", "123456789012"),
        KeyValue::new("faas.coldstart", Value::Bool(true)),
    ];

    SpanData {
        span_context,
        parent_span_id: parent_span_id.unwrap_or(SpanId::INVALID),
        parent_span_is_remote: parent_span_id.is_some(),
        span_kind: SpanKind::Server,
        name: "my-lambda-function".into(),
        start_time,
        end_time,
        attributes,
        dropped_attributes_count: 0,
        events: SpanEvents::default(),
        links: SpanLinks::default(),
        status: Status::Unset,
        instrumentation_scope: InstrumentationScope::builder("lambda-runtime").build(),
    }
}

/// Creates a DynamoDB span (Client span).
pub fn create_dynamodb_span(
    trace_id: TraceId,
    span_id: SpanId,
    parent_span_id: SpanId,
) -> SpanData {
    let span_context = SpanContext::new(
        trace_id,
        span_id,
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    );

    let start_time = UNIX_EPOCH + Duration::from_millis(1700000000010);
    let end_time = start_time + Duration::from_millis(50);

    let attributes = vec![
        KeyValue::new("rpc.service", "DynamoDB"),
        KeyValue::new("rpc.method", "PutItem"),
        KeyValue::new("rpc.system", "aws-api"),
        KeyValue::new("cloud.region", "us-east-1"),
        KeyValue::new("db.system", "dynamodb"),
        KeyValue::new(
            "aws.dynamodb.table_names",
            Value::Array(opentelemetry::Array::String(vec![
                opentelemetry::StringValue::from("my-table"),
            ])),
        ),
        KeyValue::new("aws.request_id", "ABCD1234EFGH5678"),
    ];

    SpanData {
        span_context,
        parent_span_id,
        parent_span_is_remote: false,
        span_kind: SpanKind::Client,
        name: "DynamoDB.PutItem".into(),
        start_time,
        end_time,
        attributes,
        dropped_attributes_count: 0,
        events: SpanEvents::default(),
        links: SpanLinks::default(),
        status: Status::Ok,
        instrumentation_scope: InstrumentationScope::builder("aws-sdk").build(),
    }
}

/// Creates an HTTP client span.
pub fn create_http_client_span(
    trace_id: TraceId,
    span_id: SpanId,
    parent_span_id: SpanId,
    status_code: i64,
) -> SpanData {
    let span_context = SpanContext::new(
        trace_id,
        span_id,
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    );

    let start_time = UNIX_EPOCH + Duration::from_millis(1700000000020);
    let end_time = start_time + Duration::from_millis(75);

    let attributes = vec![
        KeyValue::new("http.method", "GET"),
        KeyValue::new("http.url", "https://api.example.com/users/123"),
        KeyValue::new("http.status_code", Value::I64(status_code)),
        KeyValue::new("http.request.header.user_agent", "MyApp/1.0"),
        KeyValue::new("http.response.header.content_type", "application/json"),
        KeyValue::new("net.peer.name", "api.example.com"),
        KeyValue::new("net.peer.port", Value::I64(443)),
    ];

    SpanData {
        span_context,
        parent_span_id,
        parent_span_is_remote: false,
        span_kind: SpanKind::Client,
        name: "GET /users/:id".into(),
        start_time,
        end_time,
        attributes,
        dropped_attributes_count: 0,
        events: SpanEvents::default(),
        links: SpanLinks::default(),
        status: if status_code >= 400 {
            Status::error("")
        } else {
            Status::Ok
        },
        instrumentation_scope: InstrumentationScope::builder("http-client").build(),
    }
}
