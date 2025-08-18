#![allow(unsafe_op_in_unsafe_fn)]
#![allow(unknown_lints)]
#![allow(unsafe_attr_outside_unsafe)]

use std::ffi::CStr;
use std::os::raw::c_char;

use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValueValue;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};
use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use opentelemetry_proto::tonic::resource::v1::Resource;
use prost::Message;
use std::time::{SystemTime, UNIX_EPOCH};

pub mod builder;

#[no_mangle]
unsafe extern "C" fn geneva_build_otlp_logs_minimal(
    body_utf8: *const c_char,
    resource_key: *const c_char,
    resource_value: *const c_char,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    // Return codes aligned with GenevaError:
    // 0 = GENEVA_SUCCESS
    // 4 = GENEVA_INVALID_DATA
    // 100 = GENEVA_ERR_NULL_POINTER

    if out_ptr.is_null() || out_len.is_null() {
        return 100;
    }
    *out_ptr = std::ptr::null_mut();
    *out_len = 0;

    if body_utf8.is_null() {
        return 100;
    }

    let body = match CStr::from_ptr(body_utf8).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return 4,
    };

    let mut resource_attrs: Vec<KeyValue> = Vec::new();
    if !resource_key.is_null() && !resource_value.is_null() {
        let key = match CStr::from_ptr(resource_key).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return 4,
        };
        let val = match CStr::from_ptr(resource_value).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return 4,
        };
        resource_attrs.push(KeyValue {
            key,
            value: Some(AnyValue {
                value: Some(AnyValueValue::StringValue(val)),
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
        event_name: "Log".to_string(),
        body: Some(AnyValue {
            value: Some(AnyValueValue::StringValue(body)),
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

    let mut bytes = req.encode_to_vec();
    let len = bytes.len();
    let ptr = bytes.as_mut_ptr();
    std::mem::forget(bytes);

    *out_ptr = ptr;
    *out_len = len;
    0
}

#[no_mangle]
unsafe extern "C" fn geneva_free_buffer(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len > 0 {
        let _ = Vec::from_raw_parts(ptr, len, len);
    }
}
