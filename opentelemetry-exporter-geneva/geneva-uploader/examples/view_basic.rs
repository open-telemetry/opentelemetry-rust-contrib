//! End-to-end example: send OTLP log bytes via the `LogsDataView` path to Geneva.
//!
//! This is the typical direct-usage pattern for callers that already have OTLP
//! protobuf bytes. The bytes are wrapped with `RawLogsData`, which implements
//! `LogsDataView` without decoding them into owned OTLP structs first.
//!
//! If you need to hand-implement the view traits for a custom in-memory model,
//! see `view_advanced.rs`.
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
//! cargo run --example view_basic -p geneva-uploader
//! ```

use geneva_uploader::client::{GenevaClient, GenevaClientConfig};
use geneva_uploader::AuthMethod;
use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::common::v1::{any_value, AnyValue, InstrumentationScope, KeyValue};
use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use opentelemetry_proto::tonic::resource::v1::Resource;
use otap_df_pdata::views::otlp::bytes::logs::RawLogsData;
use prost::Message;
use std::env;
use std::path::PathBuf;

fn build_export_logs_request() -> ExportLogsServiceRequest {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;

    ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            resource: Some(Resource {
                attributes: vec![KeyValue {
                    key: "service.name".to_string(),
                    value: Some(AnyValue {
                        value: Some(any_value::Value::StringValue(
                            "geneva-view-basic-example".to_string(),
                        )),
                    }),
                }],
                ..Default::default()
            }),
            scope_logs: vec![ScopeLogs {
                scope: Some(InstrumentationScope {
                    name: "geneva-uploader.examples.view_basic".to_string(),
                    ..Default::default()
                }),
                log_records: vec![
                    LogRecord {
                        time_unix_nano: now,
                        observed_time_unix_nano: now,
                        severity_number: 9,
                        body: Some(AnyValue {
                            value: Some(any_value::Value::StringValue(
                                "hello from view path".to_string(),
                            )),
                        }),
                        ..Default::default()
                    },
                    LogRecord {
                        time_unix_nano: now + 1_000_000,
                        observed_time_unix_nano: now + 1_000_000,
                        severity_number: 9,
                        body: Some(AnyValue {
                            value: Some(any_value::Value::StringValue(
                                "second view log".to_string(),
                            )),
                        }),
                        ..Default::default()
                    },
                    LogRecord {
                        time_unix_nano: now + 2_000_000,
                        observed_time_unix_nano: now + 2_000_000,
                        severity_number: 17,
                        body: Some(AnyValue {
                            value: Some(any_value::Value::StringValue(
                                "view alert fired".to_string(),
                            )),
                        }),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
}

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
        obo_event_map: None,
    })
    .expect("Failed to create GenevaClient");

    let bytes = build_export_logs_request().encode_to_vec();
    let view = RawLogsData::new(&bytes);
    let batches = client
        .encode_and_compress_logs(&view)
        .expect("Encoding failed");

    println!("Encoded {} batch(es):", batches.len());
    for batch in &batches {
        println!("  event_name={} rows={}", batch.event_name, batch.row_count,);
    }

    for batch in &batches {
        client.upload_batch(batch).await.expect("Upload failed");
        println!("  uploaded batch: {}", batch.event_name);
    }

    println!("Done. Check Geneva for event 'Log' in your namespace.");
}
