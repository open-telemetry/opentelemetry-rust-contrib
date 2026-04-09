//! End-to-end example: send logs via a hand-written `LogsDataView` to Geneva.
//!
//! This exercises `GenevaClient::encode_and_compress_logs` + `upload_batch`
//! without going through the OpenTelemetry SDK.
//!
//! This example is intentionally verbose. It demonstrates the full trait
//! surface required to hand-implement `LogsDataView` for a custom in-memory
//! representation. Most callers should start with `view_basic.rs` instead:
//! Rust SDK users go through `GenevaExporter`, OTLP-byte callers can wrap
//! protobuf payloads with `RawLogsData`, and otap-dataflow users already get
//! the pdata view implementations from `otel-arrow`.
//!
//! # Required environment variables
//!
//! ```bash
//! export GENEVA_ENDPOINT="https://<your-gcs-endpoint>"
//! export GENEVA_ENVIRONMENT="Test"
//! export GENEVA_ACCOUNT="<account>"
//! export GENEVA_NAMESPACE="<namespace>"
//! export GENEVA_REGION="eastus"
//! export GENEVA_CERT_PATH="/path/to/client.p12"
//! export GENEVA_CERT_PASSWORD="<password>"
//! export GENEVA_CONFIG_MAJOR_VERSION=2
//! # Optional
//! export GENEVA_TENANT="default-tenant"
//! export GENEVA_ROLE_NAME="default-role"
//! export GENEVA_ROLE_INSTANCE="default-instance"
//! ```
//!
//! # Run
//!
//! ```bash
//! cargo run --example view_advanced -p geneva-uploader
//! ```
//!
//! If the upload succeeds you will see the encoded batch sizes printed and the
//! records will appear in Geneva under the event name `"Log"`.

use geneva_uploader::client::{GenevaClient, GenevaClientConfig};
use geneva_uploader::AuthMethod;
use otap_df_pdata_views::views::{
    common::{AnyValueView, AttributeView, InstrumentationScopeView, ValueType},
    logs::{LogRecordView, LogsDataView, ResourceLogsView, ScopeLogsView},
    resource::ResourceView,
};
use std::env;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Minimal LogsDataView implementation
// ---------------------------------------------------------------------------

struct SimpleLogRecord {
    event_name: Vec<u8>,
    severity_number: i32,
    observed_time_unix_nano: u64,
    body: Vec<u8>,
}

struct SimpleScopeLogs {
    records: Vec<SimpleLogRecord>,
}

struct SimpleResourceLogs {
    scopes: Vec<SimpleScopeLogs>,
}

struct SimpleLogsData {
    resources: Vec<SimpleResourceLogs>,
}

// --- AnyValue (body only; we only need String) ---

struct BodyValue<'a>(&'a [u8]);

impl<'a> AnyValueView<'a> for BodyValue<'a> {
    type KeyValue = NoAttr;
    type ArrayIter<'arr>
        = std::iter::Empty<Self>
    where
        Self: 'arr;
    type KeyValueIter<'kv>
        = std::iter::Empty<NoAttr>
    where
        Self: 'kv;

    fn value_type(&self) -> ValueType {
        ValueType::String
    }
    fn as_string(&self) -> Option<&[u8]> {
        Some(self.0)
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

// --- Stubs for unused associated types ---

struct NoAttr;

impl AttributeView for NoAttr {
    type Val<'v>
        = BodyValue<'v>
    where
        Self: 'v;
    fn key(&self) -> &[u8] {
        b""
    }
    fn value(&self) -> Option<Self::Val<'_>> {
        None
    }
}

struct NoResource;
impl ResourceView for NoResource {
    type Attribute<'a>
        = NoAttr
    where
        Self: 'a;
    type AttributesIter<'a>
        = std::iter::Empty<NoAttr>
    where
        Self: 'a;
    fn attributes(&self) -> Self::AttributesIter<'_> {
        std::iter::empty()
    }
    fn dropped_attributes_count(&self) -> u32 {
        0
    }
}

struct NoScope;
impl InstrumentationScopeView for NoScope {
    type Attribute<'a>
        = NoAttr
    where
        Self: 'a;
    type AttributeIter<'a>
        = std::iter::Empty<NoAttr>
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

// --- LogRecordView ---

struct SimpleLogRecordRef<'a>(&'a SimpleLogRecord);

impl<'a> LogRecordView for SimpleLogRecordRef<'a> {
    type Attribute<'att>
        = NoAttr
    where
        Self: 'att;
    type AttributeIter<'att>
        = std::iter::Empty<NoAttr>
    where
        Self: 'att;
    type Body<'bod>
        = BodyValue<'bod>
    where
        Self: 'bod;

    fn time_unix_nano(&self) -> Option<u64> {
        None
    }
    fn observed_time_unix_nano(&self) -> Option<u64> {
        Some(self.0.observed_time_unix_nano)
    }
    fn severity_number(&self) -> Option<i32> {
        Some(self.0.severity_number)
    }
    fn severity_text(&self) -> Option<&[u8]> {
        None
    }
    fn body(&self) -> Option<Self::Body<'_>> {
        Some(BodyValue(&self.0.body))
    }
    fn attributes(&self) -> Self::AttributeIter<'_> {
        std::iter::empty()
    }
    fn dropped_attributes_count(&self) -> u32 {
        0
    }
    fn flags(&self) -> Option<u32> {
        None
    }
    fn trace_id(&self) -> Option<&[u8; 16]> {
        None
    }
    fn span_id(&self) -> Option<&[u8; 8]> {
        None
    }
    fn event_name(&self) -> Option<&[u8]> {
        Some(&self.0.event_name)
    }
}

// --- ScopeLogsView ---

struct SimpleScopeLogsRef<'a>(&'a SimpleScopeLogs);

impl<'a> ScopeLogsView for SimpleScopeLogsRef<'a> {
    type Scope<'s>
        = NoScope
    where
        Self: 's;
    type LogRecord<'r>
        = SimpleLogRecordRef<'r>
    where
        Self: 'r;
    type LogRecordsIter<'r>
        = std::iter::Map<
        std::slice::Iter<'r, SimpleLogRecord>,
        fn(&'r SimpleLogRecord) -> SimpleLogRecordRef<'r>,
    >
    where
        Self: 'r;

    fn scope(&self) -> Option<Self::Scope<'_>> {
        None
    }
    fn log_records(&self) -> Self::LogRecordsIter<'_> {
        self.0.records.iter().map(SimpleLogRecordRef)
    }
    fn schema_url(&self) -> Option<&[u8]> {
        None
    }
}

// --- ResourceLogsView ---

struct SimpleResourceLogsRef<'a>(&'a SimpleResourceLogs);

impl<'a> ResourceLogsView for SimpleResourceLogsRef<'a> {
    type Resource<'r>
        = NoResource
    where
        Self: 'r;
    type ScopeLogs<'s>
        = SimpleScopeLogsRef<'s>
    where
        Self: 's;
    type ScopesIter<'s>
        = std::iter::Map<
        std::slice::Iter<'s, SimpleScopeLogs>,
        fn(&'s SimpleScopeLogs) -> SimpleScopeLogsRef<'s>,
    >
    where
        Self: 's;

    fn resource(&self) -> Option<Self::Resource<'_>> {
        None
    }
    fn scopes(&self) -> Self::ScopesIter<'_> {
        self.0.scopes.iter().map(SimpleScopeLogsRef)
    }
    fn schema_url(&self) -> Option<&[u8]> {
        None
    }
}

// --- LogsDataView ---

impl LogsDataView for SimpleLogsData {
    type ResourceLogs<'r>
        = SimpleResourceLogsRef<'r>
    where
        Self: 'r;
    type ResourcesIter<'r>
        = std::iter::Map<
        std::slice::Iter<'r, SimpleResourceLogs>,
        fn(&'r SimpleResourceLogs) -> SimpleResourceLogsRef<'r>,
    >
    where
        Self: 'r;

    fn resources(&self) -> Self::ResourcesIter<'_> {
        self.resources.iter().map(SimpleResourceLogsRef)
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let endpoint = env::var("GENEVA_ENDPOINT").expect("GENEVA_ENDPOINT required");
    let environment = env::var("GENEVA_ENVIRONMENT").expect("GENEVA_ENVIRONMENT required");
    let account = env::var("GENEVA_ACCOUNT").expect("GENEVA_ACCOUNT required");
    let namespace = env::var("GENEVA_NAMESPACE").expect("GENEVA_NAMESPACE required");
    let region = env::var("GENEVA_REGION").expect("GENEVA_REGION required");
    let cert_path = PathBuf::from(env::var("GENEVA_CERT_PATH").expect("GENEVA_CERT_PATH required"));
    let cert_password = env::var("GENEVA_CERT_PASSWORD").expect("GENEVA_CERT_PASSWORD required");
    let config_major_version: u32 = env::var("GENEVA_CONFIG_MAJOR_VERSION")
        .expect("GENEVA_CONFIG_MAJOR_VERSION required")
        .parse()
        .expect("GENEVA_CONFIG_MAJOR_VERSION must be a u32");

    let tenant = env::var("GENEVA_TENANT").unwrap_or_else(|_| "default-tenant".to_string());
    let role_name = env::var("GENEVA_ROLE_NAME").unwrap_or_else(|_| "default-role".to_string());
    let role_instance =
        env::var("GENEVA_ROLE_INSTANCE").unwrap_or_else(|_| "default-instance".to_string());

    let client = GenevaClient::new(GenevaClientConfig {
        endpoint,
        environment,
        account,
        namespace,
        region,
        config_major_version,
        auth_method: AuthMethod::Certificate {
            path: cert_path,
            password: cert_password,
        },
        tenant,
        role_name,
        role_instance,
        msi_resource: None,
    })
    .expect("Failed to create GenevaClient");

    // Build a simple LogsDataView with two event names.
    // In real usage this would be a view over otap-dataflow's Arrow-backed pdata.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;

    let view = SimpleLogsData {
        resources: vec![SimpleResourceLogs {
            scopes: vec![SimpleScopeLogs {
                records: vec![
                    SimpleLogRecord {
                        event_name: b"Log".to_vec(),
                        severity_number: 9, // INFO
                        observed_time_unix_nano: now,
                        body: b"hello from view path".to_vec(),
                    },
                    SimpleLogRecord {
                        event_name: b"Log".to_vec(),
                        severity_number: 9,
                        observed_time_unix_nano: now + 1_000_000,
                        body: b"second view log".to_vec(),
                    },
                    SimpleLogRecord {
                        event_name: b"Log".to_vec(),
                        severity_number: 17, // ERROR
                        observed_time_unix_nano: now + 2_000_000,
                        body: b"view alert fired".to_vec(),
                    },
                ],
            }],
        }],
    };

    // Encode via the view path
    let batches = client
        .encode_and_compress_logs(&view)
        .expect("Encoding failed");

    println!("Encoded {} batch(es):", batches.len());
    for batch in &batches {
        println!(
            "  event_name={} rows={} compressed_bytes={}",
            batch.event_name,
            batch.row_count,
            batch.data.len()
        );
    }

    // Upload each batch
    for batch in &batches {
        client.upload_batch(batch).await.expect("Upload failed");
        println!("  uploaded batch: {}", batch.event_name);
    }

    println!("Done. Check Geneva for event 'Log' in your namespace.");
}
