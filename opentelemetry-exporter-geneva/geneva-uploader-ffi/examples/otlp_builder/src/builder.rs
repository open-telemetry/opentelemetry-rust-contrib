use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValueValue;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};
use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use opentelemetry_proto::tonic::resource::v1::Resource;
use prost::Message;
use std::time::{SystemTime, UNIX_EPOCH};

/// Pure Rust helper to build a minimal OTLP ExportLogsServiceRequest as bytes.
/// This is shared by the C example dylib and test-only usage via include! from lib.rs tests.
///
/// **Note**: This function is only intended for examples and unit tests, not for external use.
pub fn build_otlp_logs_minimal(
    event_name: &str,
    body: &str,
    resource_kv: Option<(&str, &str)>,
) -> Vec<u8> {
    let mut resource_attrs: Vec<KeyValue> = Vec::new();
    if let Some((k, v)) = resource_kv {
        resource_attrs.push(KeyValue {
            key: k.to_string(),
            value: Some(AnyValue {
                value: Some(AnyValueValue::StringValue(v.to_string())),
            }),
        });
    }

    let now_nanos: u64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;

    let log_record = LogRecord {
        time_unix_nano: now_nanos,
        observed_time_unix_nano: 0,
        severity_number: 0,
        severity_text: String::new(),
        event_name: event_name.to_string(),
        body: Some(AnyValue {
            value: Some(AnyValueValue::StringValue(body.to_string())),
        }),
        attributes: Vec::new(),
        dropped_attributes_count: 0,
        flags: 0,
        trace_id: Vec::new(),
        span_id: Vec::new(),
    };

    let scope_logs = ScopeLogs {
        scope: None,
        log_records: vec![log_record],
        schema_url: String::new(),
    };

    let resource_logs = ResourceLogs {
        resource: Some(Resource {
            attributes: resource_attrs,
            dropped_attributes_count: 0,
            entity_refs: Vec::new(),
        }),
        scope_logs: vec![scope_logs],
        schema_url: String::new(),
    };

    let req = ExportLogsServiceRequest {
        resource_logs: vec![resource_logs],
    };

    req.encode_to_vec()
}
