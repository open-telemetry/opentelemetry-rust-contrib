use core::fmt;
use futures::stream::{self, StreamExt};
use geneva_uploader::client::GenevaClient;
use opentelemetry_proto::tonic::common::v1::{any_value::Value as ProtoVal, AnyValue, KeyValue};
use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use opentelemetry_proto::transform::common::tonic::ResourceAttributesWithSchema;
use opentelemetry_proto::transform::logs::tonic::group_logs_by_resource_and_scope;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::logs::LogBatch;
use otap_df_pdata_views::views::{
    common::{AnyValueView, AttributeView, InstrumentationScopeView, ValueType},
    logs::{LogRecordView, LogsDataView, ResourceLogsView, ScopeLogsView},
    resource::ResourceView,
};
use std::sync::{atomic, Arc};

/// An OpenTelemetry exporter that writes logs to Geneva exporter
pub struct GenevaExporter {
    resource: ResourceAttributesWithSchema,
    _is_shutdown: atomic::AtomicBool,
    geneva_client: Arc<GenevaClient>,
    max_concurrent_uploads: usize,
}

// TODO - Add builder pattern for GenevaExporter to allow more flexible configuration
impl GenevaExporter {
    /// Create a new GenavaExporter
    pub fn new(geneva_client: GenevaClient) -> Self {
        Self::new_with_concurrency(geneva_client, 4) // Default to 4 concurrent uploads
    }

    /// Create a new GenavaExporter with custom concurrency level
    pub fn new_with_concurrency(
        geneva_client: GenevaClient,
        max_concurrent_uploads: usize,
    ) -> Self {
        Self {
            resource: ResourceAttributesWithSchema::default(),
            _is_shutdown: atomic::AtomicBool::new(false),
            geneva_client: Arc::new(geneva_client),
            max_concurrent_uploads,
        }
    }
}

impl fmt::Debug for GenevaExporter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Genava exporter")
    }
}

impl opentelemetry_sdk::logs::LogExporter for GenevaExporter {
    async fn export(&self, batch: LogBatch<'_>) -> OTelSdkResult {
        let otlp = group_logs_by_resource_and_scope(batch, &self.resource);

        // Encode and compress logs into batches
        let view = ProtoLogsView(&otlp);
        let compressed_batches = match self.geneva_client.encode_and_compress_logs(&view) {
            Ok(batches) => batches,
            Err(e) => return Err(OTelSdkError::InternalFailure(e)),
        };

        // Execute uploads concurrently within the same async task using buffer_unordered.
        // This processes up to max_concurrent_uploads batches simultaneously without
        // spawning new tasks or threads, using async I/O concurrency instead.
        // All batch uploads are processed asynchronously in the same task context that
        // called the export() method.
        let errors: Vec<String> = stream::iter(compressed_batches)
            .map(|batch| {
                let client = self.geneva_client.clone();
                async move { client.upload_batch(&batch).await }
            })
            .buffer_unordered(self.max_concurrent_uploads)
            .filter_map(|result| async move { result.err() })
            .collect()
            .await;

        // Return error if any uploads failed
        if !errors.is_empty() {
            return Err(OTelSdkError::InternalFailure(format!(
                "Upload failures: {}",
                errors.join("; ")
            )));
        }

        Ok(())
    }

    fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
        self.resource = resource.into();
    }
}

// ---------------------------------------------------------------------------
// ProtoLogsView: LogsDataView impl for OTLP proto ResourceLogs
// Wraps `&[ResourceLogs]` directly, avoiding proto→bytes→parse round-trip.
// ---------------------------------------------------------------------------

struct ProtoLogsView<'a>(&'a [ResourceLogs]);

impl<'a> LogsDataView for ProtoLogsView<'a> {
    type ResourceLogs<'r>
        = ProtoRLView<'r>
    where
        Self: 'r;
    type ResourcesIter<'r>
        = ProtoRLIter<'r>
    where
        Self: 'r;
    fn resources(&self) -> Self::ResourcesIter<'_> {
        ProtoRLIter(self.0.iter())
    }
}

struct ProtoRLIter<'a>(std::slice::Iter<'a, ResourceLogs>);
impl<'a> Iterator for ProtoRLIter<'a> {
    type Item = ProtoRLView<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(ProtoRLView)
    }
}

struct ProtoRLView<'a>(&'a ResourceLogs);
impl<'a> ResourceLogsView for ProtoRLView<'a> {
    type Resource<'r>
        = NoView
    where
        Self: 'r;
    type ScopeLogs<'s>
        = ProtoSLView<'s>
    where
        Self: 's;
    type ScopesIter<'s>
        = ProtoSLIter<'s>
    where
        Self: 's;
    fn resource(&self) -> Option<Self::Resource<'_>> {
        None
    }
    fn scopes(&self) -> Self::ScopesIter<'_> {
        ProtoSLIter(self.0.scope_logs.iter())
    }
    fn schema_url(&self) -> Option<&[u8]> {
        if self.0.schema_url.is_empty() {
            None
        } else {
            Some(self.0.schema_url.as_bytes())
        }
    }
}

struct ProtoSLIter<'a>(std::slice::Iter<'a, ScopeLogs>);
impl<'a> Iterator for ProtoSLIter<'a> {
    type Item = ProtoSLView<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(ProtoSLView)
    }
}

struct ProtoSLView<'a>(&'a ScopeLogs);
impl<'a> ScopeLogsView for ProtoSLView<'a> {
    type Scope<'s>
        = NoView
    where
        Self: 's;
    type LogRecord<'r>
        = ProtoLRView<'r>
    where
        Self: 'r;
    type LogRecordsIter<'r>
        = ProtoLRIter<'r>
    where
        Self: 'r;
    fn scope(&self) -> Option<Self::Scope<'_>> {
        None
    }
    fn log_records(&self) -> Self::LogRecordsIter<'_> {
        ProtoLRIter(self.0.log_records.iter())
    }
    fn schema_url(&self) -> Option<&[u8]> {
        if self.0.schema_url.is_empty() {
            None
        } else {
            Some(self.0.schema_url.as_bytes())
        }
    }
}

struct ProtoLRIter<'a>(std::slice::Iter<'a, LogRecord>);
impl<'a> Iterator for ProtoLRIter<'a> {
    type Item = ProtoLRView<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(ProtoLRView)
    }
}

struct ProtoLRView<'a>(&'a LogRecord);
impl<'a> LogRecordView for ProtoLRView<'a> {
    type Attribute<'att>
        = ProtoKVView<'att>
    where
        Self: 'att;
    type AttributeIter<'att>
        = ProtoKVIter<'att>
    where
        Self: 'att;
    type Body<'bod>
        = ProtoAVView<'bod>
    where
        Self: 'bod;
    fn time_unix_nano(&self) -> Option<u64> {
        if self.0.time_unix_nano != 0 {
            Some(self.0.time_unix_nano)
        } else {
            None
        }
    }
    fn observed_time_unix_nano(&self) -> Option<u64> {
        if self.0.observed_time_unix_nano != 0 {
            Some(self.0.observed_time_unix_nano)
        } else {
            None
        }
    }
    fn severity_number(&self) -> Option<i32> {
        if self.0.severity_number != 0 {
            Some(self.0.severity_number)
        } else {
            None
        }
    }
    fn severity_text(&self) -> Option<&[u8]> {
        if self.0.severity_text.is_empty() {
            None
        } else {
            Some(self.0.severity_text.as_bytes())
        }
    }
    fn body(&self) -> Option<Self::Body<'_>> {
        self.0.body.as_ref().map(ProtoAVView)
    }
    fn attributes(&self) -> Self::AttributeIter<'_> {
        ProtoKVIter(self.0.attributes.iter())
    }
    fn dropped_attributes_count(&self) -> u32 {
        self.0.dropped_attributes_count
    }
    fn flags(&self) -> Option<u32> {
        if self.0.flags != 0 {
            Some(self.0.flags)
        } else {
            None
        }
    }
    fn trace_id(&self) -> Option<&[u8; 16]> {
        let id = <&[u8; 16]>::try_from(self.0.trace_id.as_slice()).ok()?;
        if id == &[0u8; 16] {
            None
        } else {
            Some(id)
        }
    }
    fn span_id(&self) -> Option<&[u8; 8]> {
        let id = <&[u8; 8]>::try_from(self.0.span_id.as_slice()).ok()?;
        if id == &[0u8; 8] {
            None
        } else {
            Some(id)
        }
    }
    fn event_name(&self) -> Option<&[u8]> {
        if self.0.event_name.is_empty() {
            None
        } else {
            Some(self.0.event_name.as_bytes())
        }
    }
}

struct ProtoAVView<'a>(&'a AnyValue);
impl<'a> AnyValueView<'a> for ProtoAVView<'a> {
    type KeyValue = ProtoKVView<'a>;
    type ArrayIter<'arr>
        = std::iter::Empty<ProtoAVView<'a>>
    where
        Self: 'arr;
    type KeyValueIter<'kv>
        = std::iter::Empty<ProtoKVView<'a>>
    where
        Self: 'kv;
    fn value_type(&self) -> ValueType {
        match &self.0.value {
            Some(ProtoVal::StringValue(_)) => ValueType::String,
            Some(ProtoVal::BoolValue(_)) => ValueType::Bool,
            Some(ProtoVal::IntValue(_)) => ValueType::Int64,
            Some(ProtoVal::DoubleValue(_)) => ValueType::Double,
            Some(ProtoVal::ArrayValue(_)) => ValueType::Empty, // not iterable through this view
            Some(ProtoVal::KvlistValue(_)) => ValueType::Empty, // not iterable through this view
            Some(ProtoVal::BytesValue(_)) => ValueType::Bytes,
            None => ValueType::Empty,
        }
    }
    fn as_string(&self) -> Option<&[u8]> {
        if let Some(ProtoVal::StringValue(s)) = &self.0.value {
            Some(s.as_bytes())
        } else {
            None
        }
    }
    fn as_bool(&self) -> Option<bool> {
        if let Some(ProtoVal::BoolValue(b)) = &self.0.value {
            Some(*b)
        } else {
            None
        }
    }
    fn as_int64(&self) -> Option<i64> {
        if let Some(ProtoVal::IntValue(i)) = &self.0.value {
            Some(*i)
        } else {
            None
        }
    }
    fn as_double(&self) -> Option<f64> {
        if let Some(ProtoVal::DoubleValue(d)) = &self.0.value {
            Some(*d)
        } else {
            None
        }
    }
    fn as_bytes(&self) -> Option<&[u8]> {
        if let Some(ProtoVal::BytesValue(b)) = &self.0.value {
            Some(b)
        } else {
            None
        }
    }
    fn as_array(&self) -> Option<Self::ArrayIter<'_>> {
        None
    }
    fn as_kvlist(&self) -> Option<Self::KeyValueIter<'_>> {
        None
    }
}

struct ProtoKVView<'a>(&'a KeyValue);
struct ProtoKVIter<'a>(std::slice::Iter<'a, KeyValue>);
impl<'a> Iterator for ProtoKVIter<'a> {
    type Item = ProtoKVView<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(ProtoKVView)
    }
}
impl<'a> AttributeView for ProtoKVView<'a> {
    type Val<'val>
        = ProtoAVView<'val>
    where
        Self: 'val;
    fn key(&self) -> &[u8] {
        self.0.key.as_bytes()
    }
    fn value(&self) -> Option<Self::Val<'_>> {
        self.0.value.as_ref().map(ProtoAVView)
    }
}

// ---------------------------------------------------------------------------
// Stub view types (no resource/scope metadata in proto path)
// ---------------------------------------------------------------------------

struct NoView;

impl ResourceView for NoView {
    type Attribute<'a>
        = NoAttrView
    where
        Self: 'a;
    type AttributesIter<'a>
        = std::iter::Empty<NoAttrView>
    where
        Self: 'a;
    fn attributes(&self) -> Self::AttributesIter<'_> {
        std::iter::empty()
    }
    fn dropped_attributes_count(&self) -> u32 {
        0
    }
}

impl InstrumentationScopeView for NoView {
    type Attribute<'a>
        = NoAttrView
    where
        Self: 'a;
    type AttributeIter<'a>
        = std::iter::Empty<NoAttrView>
    where
        Self: 'a;
    fn name(&self) -> Option<&[u8]> {
        None
    }
    fn version(&self) -> Option<&[u8]> {
        None
    }
    fn attributes(&self) -> Self::AttributeIter<'_> {
        std::iter::empty()
    }
    fn dropped_attributes_count(&self) -> u32 {
        0
    }
}

struct NoAttrView;
impl AttributeView for NoAttrView {
    type Val<'val>
        = NoAnyValue
    where
        Self: 'val;
    fn key(&self) -> &[u8] {
        b""
    }
    fn value(&self) -> Option<Self::Val<'_>> {
        None
    }
}

struct NoAnyValue;
impl<'a> AnyValueView<'a> for NoAnyValue {
    type KeyValue = NoAttrView;
    type ArrayIter<'arr>
        = std::iter::Empty<NoAnyValue>
    where
        Self: 'arr;
    type KeyValueIter<'kv>
        = std::iter::Empty<NoAttrView>
    where
        Self: 'kv;
    fn value_type(&self) -> ValueType {
        ValueType::Empty
    }
    fn as_string(&self) -> Option<&[u8]> {
        None
    }
    fn as_bool(&self) -> Option<bool> {
        None
    }
    fn as_int64(&self) -> Option<i64> {
        None
    }
    fn as_double(&self) -> Option<f64> {
        None
    }
    fn as_bytes(&self) -> Option<&[u8]> {
        None
    }
    fn as_array(&self) -> Option<Self::ArrayIter<'_>> {
        None
    }
    fn as_kvlist(&self) -> Option<Self::KeyValueIter<'_>> {
        None
    }
}
