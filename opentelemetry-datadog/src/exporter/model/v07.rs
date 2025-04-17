use crate::exporter::model::{DD_MEASURED_KEY, SAMPLING_PRIORITY_KEY};
use crate::exporter::{Error, ModelConfig};
use opentelemetry::trace::{Event, Status};
use opentelemetry::KeyValue;
use opentelemetry_sdk::trace::SpanData;
use opentelemetry_sdk::Resource;
use rmp::encode::ValueWriteError;
use std::time::SystemTime;

use super::unified_tags::UnifiedTags;
use super::{get_measuring, get_sampling_priority, get_span_type};

// Documentation for all versions sourced from: https://github.com/DataDog/datadog-agent/blob/main/pkg/trace/api/version.go
// Specifically, the v0.7 versions is described with a protobuf definition but is still encoded using message pack.
//
// message TraceChunk {
// 	// priority specifies sampling priority of the trace.
// 	// @gotags: json:"priority" msg:"priority"
// 	int32 priority = 1;
// 	// origin specifies origin product ("lambda", "rum", etc.) of the trace.
// 	// @gotags: json:"origin" msg:"origin"
// 	string origin = 2;
// 	// spans specifies list of containing spans.
// 	// @gotags: json:"spans" msg:"spans"
// 	repeated Span spans = 3;
// 	// tags specifies tags common in all `spans`.
// 	// @gotags: json:"tags" msg:"tags"
// 	map<string, string> tags = 4;
// 	// droppedTrace specifies whether the trace was dropped by samplers or not.
// 	// @gotags: json:"dropped_trace" msg:"dropped_trace"
// 	bool droppedTrace = 5;
// }
//
// // TracerPayload represents a payload the trace agent receives from tracers.
// message TracerPayload {
// 	// containerID specifies the ID of the container where the tracer is running on.
// 	// @gotags: json:"container_id" msg:"container_id"
// 	string containerID = 1;
// 	// languageName specifies language of the tracer.
// 	// @gotags: json:"language_name" msg:"language_name"
// 	string languageName = 2;
// 	// languageVersion specifies language version of the tracer.
// 	// @gotags: json:"language_version" msg:"language_version"
// 	string languageVersion = 3;
// 	// tracerVersion specifies version of the tracer.
// 	// @gotags: json:"tracer_version" msg:"tracer_version"
// 	string tracerVersion = 4;
// 	// runtimeID specifies V4 UUID representation of a tracer session.
// 	// @gotags: json:"runtime_id" msg:"runtime_id"
// 	string runtimeID = 5;
// 	// chunks specifies list of containing trace chunks.
// 	// @gotags: json:"chunks" msg:"chunks"
// 	repeated TraceChunk chunks = 6;
// 	// tags specifies tags common in all `chunks`.
// 	// @gotags: json:"tags" msg:"tags"
// 	map<string, string> tags = 7;
// 	// env specifies `env` tag that set with the tracer.
// 	// @gotags: json:"env" msg:"env"
// 	string env = 8;
// 	// hostname specifies hostname of where the tracer is running.
// 	// @gotags: json:"hostname" msg:"hostname"
// 	string hostname = 9;
// 	// version specifies `version` tag that set with the tracer.
// 	// @gotags: json:"app_version" msg:"app_version"
// 	string appVersion = 10;
// }

pub(crate) fn encode<S, N, R>(
    model_config: &ModelConfig,
    traces: Vec<&[SpanData]>,
    get_service_name: S,
    get_name: N,
    get_resource: R,
    unified_tags: &UnifiedTags,
    resource: Option<&Resource>,
) -> Result<Vec<u8>, Error>
where
    for<'a> S: Fn(&'a SpanData, &'a ModelConfig) -> &'a str,
    for<'a> N: Fn(&'a SpanData, &'a ModelConfig) -> &'a str,
    for<'a> R: Fn(&'a SpanData, &'a ModelConfig) -> &'a str,
{
    let mut encoded = Vec::with_capacity(traces.len() * 512);

    rmp::encode::write_map_len(
        &mut encoded,
        3 + unified_tags.env.len() + unified_tags.version.len(),
    )?;

    // note we still don't support sending the container_id, language_version, runtime_id, tracer_version, hostname
    rmp::encode::write_str(&mut encoded, "language_name")?;
    rmp::encode::write_str(&mut encoded, "rust")?;

    encode_chunks(
        &mut encoded,
        traces,
        get_name,
        get_resource,
        get_service_name,
        model_config,
        resource,
    )?;

    encode_tags(&mut encoded, unified_tags)?;

    if let Some(env) = &unified_tags.env.value {
        rmp::encode::write_str(&mut encoded, "env")?;
        rmp::encode::write_str(&mut encoded, env)?;
    }

    if let Some(version) = &unified_tags.version.value {
        rmp::encode::write_str(&mut encoded, "app_version")?;
        rmp::encode::write_str(&mut encoded, version)?;
    }

    Ok(encoded)
}

fn encode_chunks<N, R, S>(
    encoded: &mut Vec<u8>,
    traces: Vec<&[SpanData]>,
    get_name: N,
    get_resource: R,
    get_service_name: S,
    model_config: &ModelConfig,
    resource: Option<&Resource>,
) -> Result<(), Error>
where
    for<'a> N: Fn(&'a SpanData, &'a ModelConfig) -> &'a str,
    for<'a> R: Fn(&'a SpanData, &'a ModelConfig) -> &'a str,
    for<'a> S: Fn(&'a SpanData, &'a ModelConfig) -> &'a str,
{
    rmp::encode::write_str(encoded, "chunks")?;
    rmp::encode::write_array_len(encoded, traces.len() as u32)?;
    for trace in traces.into_iter() {
        rmp::encode::write_map_len(encoded, 3)?;
        // This field isn't set on spans that didn't originate from a datadog agent so we default to 1.
        // https://github.com/vectordotdev/vector/blob/3ea8c86f9461f1e3d403c3c6820fdf19b280fe75/src/sinks/datadog/traces/request_builder.rs#L289
        rmp::encode::write_str(encoded, "priority")?;
        rmp::encode::write_i32(encoded, 1)?;
        rmp::encode::write_str(encoded, "origin")?;
        rmp::encode::write_str(encoded, "")?;

        encode_spans(
            encoded,
            trace,
            &get_name,
            &get_resource,
            &get_service_name,
            model_config,
            resource,
        )?;

        // I assume the tags here are some common values that can be extracted and deduplicated.
        // maybe support this in the future?
        // rmp::encode::write_str(payload, "tags")?;

        // todo: how to find it the trace was dropped?
        // for now assume it wasn't
        // rmp::encode::write_str(payload, "dropped_trace")?;
    }

    Ok(())
}

fn encode_spans<N, R, S>(
    encoded: &mut Vec<u8>,
    trace: &[SpanData],
    get_name: N,
    get_resource: R,
    get_service_name: S,
    model_config: &ModelConfig,
    resource: Option<&Resource>,
) -> Result<(), Error>
where
    for<'a> N: Fn(&'a SpanData, &'a ModelConfig) -> &'a str,
    for<'a> R: Fn(&'a SpanData, &'a ModelConfig) -> &'a str,
    for<'a> S: Fn(&'a SpanData, &'a ModelConfig) -> &'a str,
{
    rmp::encode::write_str(encoded, "spans")?;
    rmp::encode::write_array_len(encoded, trace.len() as u32)?;
    for span in trace {
        rmp::encode::write_map_len(encoded, 14)?;

        rmp::encode::write_str(encoded, "name")?;
        rmp::encode::write_str(encoded, get_name(span, model_config))?;

        rmp::encode::write_str(encoded, "span_id")?;
        rmp::encode::write_u64(
            encoded,
            u64::from_be_bytes(span.span_context.span_id().to_bytes()),
        )?;

        rmp::encode::write_str(encoded, "trace_id")?;
        rmp::encode::write_u64(
            encoded,
            u128::from_be_bytes(span.span_context.trace_id().to_bytes()) as u64,
        )?;

        rmp::encode::write_str(encoded, "start")?;
        let start = span
            .start_time
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as i64;
        rmp::encode::write_i64(encoded, start)?;

        rmp::encode::write_str(encoded, "duration")?;
        let duration = span
            .end_time
            .duration_since(span.start_time)
            .map(|x| x.as_nanos() as i64)
            .unwrap_or(0);
        rmp::encode::write_i64(encoded, duration)?;

        rmp::encode::write_str(encoded, "parent_id")?;
        rmp::encode::write_u64(encoded, u64::from_be_bytes(span.parent_span_id.to_bytes()))?;

        rmp::encode::write_str(encoded, "service")?;
        rmp::encode::write_str(encoded, get_service_name(span, model_config))?;

        rmp::encode::write_str(encoded, "resource")?;
        rmp::encode::write_str(encoded, get_resource(span, model_config))?;

        rmp::encode::write_str(encoded, "type")?;
        let span_type = match get_span_type(span) {
            Some(value) => value.as_str(),
            None => "".into(),
        };
        rmp::encode::write_str(encoded, &span_type)?;

        rmp::encode::write_str(encoded, "error")?;
        rmp::encode::write_i32(
            encoded,
            match span.status {
                Status::Error { .. } => 1,
                _ => 0,
            },
        )?;

        rmp::encode::write_str(encoded, "meta")?;
        rmp::encode::write_map_len(
            encoded,
            (span.attributes.len() + resource.map(|r| r.len()).unwrap_or(0)) as u32,
        )?;
        if let Some(resource) = resource {
            for (key, value) in resource.iter() {
                rmp::encode::write_str(encoded, key.as_str())?;
                rmp::encode::write_str(encoded, value.as_str().as_ref())?;
            }
        }
        for kv in span.attributes.iter() {
            rmp::encode::write_str(encoded, kv.key.as_str())?;
            rmp::encode::write_str(encoded, kv.value.as_str().as_ref())?;
        }

        encode_metrics(encoded, span)?;

        // the meta struct is usually set by datadog trace libraries and not otel so we ignore and don't serialize it
        // rmp::encode::write_str(payload, "meta_struct")?;

        encode_span_links(encoded, span)?;
        encode_span_events(encoded, span)?;
    }

    Ok(())
}

fn encode_metrics(encoded: &mut Vec<u8>, span: &SpanData) -> Result<(), Error> {
    rmp::encode::write_str(encoded, "metrics")?;
    rmp::encode::write_map_len(encoded, 2)?;

    rmp::encode::write_str(encoded, SAMPLING_PRIORITY_KEY)?;
    let sampling_priority = get_sampling_priority(span);
    rmp::encode::write_f64(encoded, sampling_priority)?;

    rmp::encode::write_str(encoded, DD_MEASURED_KEY)?;
    let measuring = get_measuring(span);
    rmp::encode::write_f64(encoded, measuring)?;

    Ok(())
}

fn encode_span_events(encoded: &mut Vec<u8>, span: &SpanData) -> Result<(), Error> {
    rmp::encode::write_str(encoded, "span_events")?;
    rmp::encode::write_array_len(encoded, span.events.len() as u32)?;
    for event in span.events.iter() {
        rmp::encode::write_map_len(encoded, 3)?;
        rmp::encode::write_str(encoded, "name")?;
        rmp::encode::write_str(encoded, event.name.to_string().as_str())?;

        rmp::encode::write_str(encoded, "time_unix_nano")?;
        let timestamp = event
            .timestamp
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as i64;
        rmp::encode::write_i64(encoded, timestamp)?;

        rmp::encode::write_str(encoded, "attributes")?;
        rmp::encode::write_map_len(encoded, event.attributes.len() as u32)?;
        for kv in event.attributes.iter() {
            encode_attribute_any_value(encoded, kv)?;
        }
    }

    Ok(())
}

// https://github.com/DataDog/datadog-agent/blob/main/pkg/proto/datadog/trace/span.proto#L38
// AttributeAnyValue is used to represent any type of attribute value. AttributeAnyValue may contain a
// primitive value such as a string or integer or it may contain an arbitrary nested
// object containing arrays, key-value lists and primitives.
// message AttributeAnyValue {
//     // We implement a union manually here because Go's MessagePack generator does not support
//     // Protobuf `oneof` unions: https://github.com/tinylib/msgp/issues/184
//     // Despite this, the format represented here is binary compatible with `oneof`, if we choose
//     // to migrate to that in the future.
//     // @gotags: json:"type" msg:"type"
//     AttributeAnyValueType type = 1;

//     enum AttributeAnyValueType {
//       STRING_VALUE = 0;
//       BOOL_VALUE = 1;
//       INT_VALUE = 2;
//       DOUBLE_VALUE = 3;
//       ARRAY_VALUE = 4;
//     }
//     // @gotags: json:"string_value" msg:"string_value"
//     string string_value = 2;
//     // @gotags: json:"bool_value" msg:"bool_value"
//     bool bool_value = 3;
//     // @gotags: json:"int_value" msg:"int_value"
//     int64 int_value = 4;
//     // @gotags: json:"double_value" msg:"double_value"
//     double double_value = 5;
//     // @gotags: json:"array_value" msg:"array_value"
//     AttributeArray array_value = 6;
//   }

//   // AttributeArray is a list of AttributeArrayValue messages. We need this as a message since `oneof` in AttributeAnyValue does not allow repeated fields.
//   message AttributeArray {
//     // Array of values. The array may be empty (contain 0 elements).
//     // @gotags: json:"values" msg:"values"
//     repeated AttributeArrayValue values = 1;
//   }

//   // An element in the homogeneous AttributeArray.
//   // Compared to AttributeAnyValue, it only supports scalar values.
//   message AttributeArrayValue {
//     // We implement a union manually here because Go's MessagePack generator does not support
//     // Protobuf `oneof` unions: https://github.com/tinylib/msgp/issues/184
//     // Despite this, the format represented here is binary compatible with `oneof`, if we choose
//     // to migrate to that in the future.
//     // @gotags: json:"type" msg:"type"
//     AttributeArrayValueType type = 1;

//     enum AttributeArrayValueType {
//       STRING_VALUE = 0;
//       BOOL_VALUE = 1;
//       INT_VALUE = 2;
//       DOUBLE_VALUE = 3;
//     }

//     // @gotags: json:"string_value" msg:"string_value"
//     string string_value = 2;
//     // @gotags: json:"bool_value" msg:"bool_value"
//     bool bool_value = 3;
//     // @gotags: json:"int_value" msg:"int_value"
//     int64 int_value = 4;
//     // @gotags: json:"double_value" msg:"double_value"
//     double double_value = 5;
//   }
fn encode_attribute_any_value(encoded: &mut Vec<u8>, kv: &KeyValue) -> Result<(), Error> {
    rmp::encode::write_str(encoded, kv.key.as_str())?;

    rmp::encode::write_map_len(encoded, 2)?;

    let (enum_type, value_str) = match &kv.value {
        opentelemetry::Value::String(_) => (0, "string_value"),
        opentelemetry::Value::Bool(_) => (1, "bool_value"),
        opentelemetry::Value::I64(_) => (2, "int_value"),
        opentelemetry::Value::F64(_) => (3, "double_value"),
        opentelemetry::Value::Array(_) => (4, "array_value"),
        unknown_value => {
            return Err(Error::Other(format!(
                "Unsupported value type: {:?}",
                unknown_value
            )));
        }
    };
    rmp::encode::write_str(encoded, "type")?;
    rmp::encode::write_i32(encoded, enum_type)?;

    rmp::encode::write_str(encoded, value_str)?;
    match &kv.value {
        opentelemetry::Value::String(value) => rmp::encode::write_str(encoded, value.as_str())?,
        // I think writing bool can't fail with writing data so we convert it to the invalid marker write to match the other errors
        opentelemetry::Value::Bool(value) => rmp::encode::write_bool(encoded, *value)
            .map_err(|e| ValueWriteError::InvalidMarkerWrite(e))?,
        opentelemetry::Value::I64(value) => rmp::encode::write_i64(encoded, *value)?,
        opentelemetry::Value::F64(value) => rmp::encode::write_f64(encoded, *value)?,
        opentelemetry::Value::Array(array_value) => {
            encode_attribute_array(encoded, array_value)?;
        }
        _ => {
            return Err(Error::Other(format!(
                "Unsupported value type: {:?}",
                kv.value
            )));
        }
    }

    Ok(())
}

fn encode_attribute_array(
    encoded: &mut Vec<u8>,
    array_value: &opentelemetry::Array,
) -> Result<(), Error> {
    match array_value {
        opentelemetry::Array::String(string_values) => {
            rmp::encode::write_array_len(encoded, string_values.len() as u32)?;
            for value in string_values.iter() {
                rmp::encode::write_map_len(encoded, 2)?;

                rmp::encode::write_str(encoded, "type")?;
                rmp::encode::write_uint8(encoded, 0)?;

                rmp::encode::write_str(encoded, "string_value")?;
                rmp::encode::write_str(encoded, value.as_str())?;
            }
        }
        opentelemetry::Array::Bool(items) => {
            rmp::encode::write_array_len(encoded, items.len() as u32)?;
            for item in items.iter() {
                rmp::encode::write_map_len(encoded, 2)?;

                rmp::encode::write_str(encoded, "type")?;
                rmp::encode::write_uint8(encoded, 1)?;

                rmp::encode::write_str(encoded, "bool_value")?;
                rmp::encode::write_bool(encoded, *item)
                    .map_err(|e| ValueWriteError::InvalidMarkerWrite(e))?;
            }
        }
        opentelemetry::Array::I64(items) => {
            rmp::encode::write_array_len(encoded, items.len() as u32)?;
            for item in items.iter() {
                rmp::encode::write_map_len(encoded, 2)?;

                rmp::encode::write_str(encoded, "type")?;
                rmp::encode::write_uint8(encoded, 2)?;

                rmp::encode::write_str(encoded, "int_value")?;
                rmp::encode::write_i64(encoded, *item)?;
            }
        }
        opentelemetry::Array::F64(items) => {
            rmp::encode::write_array_len(encoded, items.len() as u32)?;
            for item in items.iter() {
                rmp::encode::write_map_len(encoded, 2)?;

                rmp::encode::write_str(encoded, "type")?;
                rmp::encode::write_uint8(encoded, 3)?;

                rmp::encode::write_str(encoded, "double_value")?;
                rmp::encode::write_f64(encoded, *item)?;
            }
        }
        unknown => {
            return Err(Error::Other(format!(
                "Unsupported array type: {:?}",
                unknown
            )))
        }
    }
    Ok(())
}

fn encode_span_links(encoded: &mut Vec<u8>, span: &SpanData) -> Result<(), Error> {
    rmp::encode::write_str(encoded, "span_links")?;
    rmp::encode::write_array_len(encoded, span.links.len() as u32)?;
    for link in span.links.as_ref() {
        rmp::encode::write_map_len(encoded, 6)?;
        rmp::encode::write_str(encoded, "trace_id")?;
        rmp::encode::write_u64(
            encoded,
            u128::from_be_bytes(link.span_context.trace_id().to_bytes()) as u64,
        )?;

        rmp::encode::write_str(encoded, "trace_id_high")?;
        rmp::encode::write_u64(
            encoded,
            (u128::from_be_bytes(link.span_context.trace_id().to_bytes()) >> 64) as u64,
        )?;
        rmp::encode::write_str(encoded, "span_id")?;
        rmp::encode::write_u64(
            encoded,
            u64::from_be_bytes(link.span_context.span_id().to_bytes()),
        )?;

        rmp::encode::write_str(encoded, "attributes")?;
        rmp::encode::write_map_len(encoded, link.attributes.len() as u32)?;
        for kv in link.attributes.iter() {
            rmp::encode::write_str(encoded, kv.key.as_str())?;
            rmp::encode::write_str(encoded, kv.value.as_str().as_ref())?;
        }
        rmp::encode::write_str(encoded, "tracestate")?;
        rmp::encode::write_str(encoded, link.span_context.trace_state().header().as_str())?;

        rmp::encode::write_str(encoded, "flags")?;
        rmp::encode::write_u8(encoded, link.span_context.trace_flags().to_u8())?;
    }

    Ok(())
}

fn encode_tags(encoded: &mut Vec<u8>, unified_tags: &UnifiedTags) -> Result<(), Error> {
    // Not too sure about this, but to support unified tagging we encode the service, version and env in the tags.
    // Some of them like service and version are also encoded explicitly in the payload in their own fields
    let length = unified_tags.service.len() + unified_tags.version.len() + unified_tags.env.len();

    rmp::encode::write_str(encoded, "tags")?;
    rmp::encode::write_map_len(encoded, length)?;
    if let Some(value) = &unified_tags.service.value {
        rmp::encode::write_str(encoded, unified_tags.service.get_tag_name())?;
        rmp::encode::write_str(encoded, value)?;
    }

    if let Some(value) = &unified_tags.version.value {
        rmp::encode::write_str(encoded, unified_tags.version.get_tag_name())?;
        rmp::encode::write_str(encoded, value)?;
    }

    if let Some(value) = &unified_tags.env.value {
        rmp::encode::write_str(encoded, unified_tags.env.get_tag_name())?;
        rmp::encode::write_str(encoded, value)?;
    }

    Ok(())
}
