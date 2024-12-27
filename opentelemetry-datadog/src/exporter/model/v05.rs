use crate::exporter::intern::StringInterner;
use crate::exporter::model::{DD_MEASURED_KEY, SAMPLING_PRIORITY_KEY};
use crate::exporter::{Error, ModelConfig};
use crate::propagator::DatadogTraceState;
use opentelemetry::trace::Status;
use opentelemetry_sdk::export::trace::SpanData;
use opentelemetry_sdk::Resource;
use std::cell::RefCell;
use std::ops::DerefMut;
use std::time::SystemTime;

use super::unified_tags::{UnifiedTagField, UnifiedTags};

const SPAN_NUM_ELEMENTS: u32 = 12;
const METRICS_LEN: u32 = 2;
const GIT_META_TAGS_COUNT: u32 = if matches!(
    (
        option_env!("DD_GIT_REPOSITORY_URL"),
        option_env!("DD_GIT_COMMIT_SHA")
    ),
    (Some(_), Some(_))
) {
    2
} else {
    0
};

// Protocol documentation sourced from https://github.com/DataDog/datadog-agent/blob/c076ea9a1ffbde4c76d35343dbc32aecbbf99cb9/pkg/trace/api/version.go
//
// The payload is an array containing exactly 2 elements:
//
// 	1. An array of all unique strings present in the payload (a dictionary referred to by index).
// 	2. An array of traces, where each trace is an array of spans. A span is encoded as an array having
// 	   exactly 12 elements, representing all span properties, in this exact order:
//
// 		 0: Service   (uint32)
// 		 1: Name      (uint32)
// 		 2: Resource  (uint32)
// 		 3: TraceID   (uint64)
// 		 4: SpanID    (uint64)
// 		 5: ParentID  (uint64)
// 		 6: Start     (int64)
// 		 7: Duration  (int64)
// 		 8: Error     (int32)
// 		 9: Meta      (map[uint32]uint32)
// 		10: Metrics   (map[uint32]float64)
// 		11: Type      (uint32)
//
// 	Considerations:
//
// 	- The "uint32" typed values in "Service", "Name", "Resource", "Type", "Meta" and "Metrics" represent
// 	  the index at which the corresponding string is found in the dictionary. If any of the values are the
// 	  empty string, then the empty string must be added into the dictionary.
//
// 	- None of the elements can be nil. If any of them are unset, they should be given their "zero-value". Here
// 	  is an example of a span with all unset values:
//
// 		 0: 0                    // Service is "" (index 0 in dictionary)
// 		 1: 0                    // Name is ""
// 		 2: 0                    // Resource is ""
// 		 3: 0                    // TraceID
// 		 4: 0                    // SpanID
// 		 5: 0                    // ParentID
// 		 6: 0                    // Start
// 		 7: 0                    // Duration
// 		 8: 0                    // Error
// 		 9: map[uint32]uint32{}  // Meta (empty map)
// 		10: map[uint32]float64{} // Metrics (empty map)
// 		11: 0                    // Type is ""
//
// 		The dictionary in this case would be []string{""}, having only the empty string at index 0.
//
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode<S, N, R, W: std::io::Write>(
    writer: &mut W,
    model_config: &ModelConfig,
    traces: Vec<&[SpanData]>,
    get_service_name: S,
    get_name: N,
    get_resource: R,
    unified_tags: &UnifiedTags,
    resource: Option<&Resource>,
) -> Result<(), Error>
where
    for<'a> S: Fn(&'a SpanData, &'a ModelConfig) -> &'a str,
    for<'a> N: Fn(&'a SpanData, &'a ModelConfig) -> &'a str,
    for<'a> R: Fn(&'a SpanData, &'a ModelConfig) -> &'a str,
{
    thread_local! {
        static TRACES_BUFFER: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(4096));
    }
    let mut interner = StringInterner::new();
    TRACES_BUFFER.with(|buffer| {
        let buffer = &mut buffer.borrow_mut();
        buffer.clear();

        encode_traces(
            buffer.deref_mut(),
            &mut interner,
            model_config,
            get_service_name,
            get_name,
            get_resource,
            &traces,
            unified_tags,
            resource,
        )?;

        rmp::encode::write_array_len(writer, 2)?;

        interner.write_dictionary(writer)?;

        writer
            .write_all(buffer)
            .map_err(|_| Error::MessagePackError)?;

        Ok(())
    })
}

fn write_unified_tags<'a, W: std::io::Write>(
    writer: &mut W,
    interner: &mut StringInterner<'a>,
    unified_tags: &'a UnifiedTags,
) -> Result<(), Error> {
    write_unified_tag(writer, interner, &unified_tags.service)?;
    write_unified_tag(writer, interner, &unified_tags.env)?;
    write_unified_tag(writer, interner, &unified_tags.version)?;
    Ok(())
}

fn write_unified_tag<'a, W: std::io::Write>(
    writer: &mut W,
    interner: &mut StringInterner<'a>,
    tag: &'a UnifiedTagField,
) -> Result<(), Error> {
    if let Some(tag_value) = &tag.value {
        rmp::encode::write_u32(writer, interner.intern(tag.get_tag_name()))?;
        rmp::encode::write_u32(writer, interner.intern(tag_value.as_str().as_ref()))?;
    }
    Ok(())
}

#[cfg(not(feature = "agent-sampling"))]
fn get_sampling_priority(_span: &SpanData) -> f64 {
    1.0
}

#[cfg(feature = "agent-sampling")]
fn get_sampling_priority(span: &SpanData) -> f64 {
    if span.span_context.trace_state().priority_sampling_enabled() {
        1.0
    } else {
        0.0
    }
}

fn get_measuring(span: &SpanData) -> f64 {
    if span.span_context.trace_state().measuring_enabled() {
        1.0
    } else {
        0.0
    }
}

#[allow(clippy::too_many_arguments)]
fn encode_traces<'interner, S, N, R, W: std::io::Write>(
    writer: &mut W,
    interner: &mut StringInterner<'interner>,
    model_config: &'interner ModelConfig,
    get_service_name: S,
    get_name: N,
    get_resource: R,
    traces: &'interner [&[SpanData]],
    unified_tags: &'interner UnifiedTags,
    resource: Option<&'interner Resource>,
) -> Result<(), Error>
where
    for<'a> S: Fn(&'a SpanData, &'a ModelConfig) -> &'a str,
    for<'a> N: Fn(&'a SpanData, &'a ModelConfig) -> &'a str,
    for<'a> R: Fn(&'a SpanData, &'a ModelConfig) -> &'a str,
{
    rmp::encode::write_array_len(writer, traces.len() as u32)?;

    for trace in traces.iter() {
        rmp::encode::write_array_len(writer, trace.len() as u32)?;

        for span in trace.iter() {
            // Safe until the year 2262 when Datadog will need to change their API
            let start = span
                .start_time
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as i64;

            let duration = span
                .end_time
                .duration_since(span.start_time)
                .map(|x| x.as_nanos() as i64)
                .unwrap_or(0);

            let mut span_type = interner.intern("");
            for kv in &span.attributes {
                if kv.key.as_str() == "span.type" {
                    span_type = interner.intern_value(&kv.value);
                    break;
                }
            }

            // Datadog span name is OpenTelemetry component name - see module docs for more information
            rmp::encode::write_array_len(writer, SPAN_NUM_ELEMENTS)?;
            rmp::encode::write_u32(
                writer,
                interner.intern(get_service_name(span, model_config)),
            )?;
            rmp::encode::write_u32(writer, interner.intern(get_name(span, model_config)))?;
            rmp::encode::write_u32(writer, interner.intern(get_resource(span, model_config)))?;
            rmp::encode::write_u64(
                writer,
                u128::from_be_bytes(span.span_context.trace_id().to_bytes()) as u64,
            )?;
            rmp::encode::write_u64(
                writer,
                u64::from_be_bytes(span.span_context.span_id().to_bytes()),
            )?;
            rmp::encode::write_u64(writer, u64::from_be_bytes(span.parent_span_id.to_bytes()))?;
            rmp::encode::write_i64(writer, start)?;
            rmp::encode::write_i64(writer, duration)?;
            rmp::encode::write_i32(
                writer,
                match span.status {
                    Status::Error { .. } => 1,
                    _ => 0,
                },
            )?;

            rmp::encode::write_map_len(
                writer,
                (span.attributes.len() + resource.map(|r| r.len()).unwrap_or(0)) as u32
                    + unified_tags.compute_attribute_size()
                    + GIT_META_TAGS_COUNT,
            )?;
            if let Some(resource) = resource {
                for (key, value) in resource.iter() {
                    rmp::encode::write_u32(writer, interner.intern(key.as_str()))?;
                    rmp::encode::write_u32(writer, interner.intern_value(value))?;
                }
            }

            write_unified_tags(writer, interner, unified_tags)?;

            for kv in span.attributes.iter() {
                rmp::encode::write_u32(writer, interner.intern(kv.key.as_str()))?;
                rmp::encode::write_u32(writer, interner.intern_value(&kv.value))?;
            }

            if let (Some(repository_url), Some(commit_sha)) = (
                option_env!("DD_GIT_REPOSITORY_URL"),
                option_env!("DD_GIT_COMMIT_SHA"),
            ) {
                rmp::encode::write_u32(writer, interner.intern("git.repository_url"))?;
                rmp::encode::write_u32(writer, interner.intern(repository_url))?;
                rmp::encode::write_u32(writer, interner.intern("git.commit.sha"))?;
                rmp::encode::write_u32(writer, interner.intern(commit_sha))?;
            }

            rmp::encode::write_map_len(writer, METRICS_LEN)?;
            rmp::encode::write_u32(writer, interner.intern(SAMPLING_PRIORITY_KEY))?;
            let sampling_priority = get_sampling_priority(span);
            rmp::encode::write_f64(writer, sampling_priority)?;

            rmp::encode::write_u32(writer, interner.intern(DD_MEASURED_KEY))?;
            let measuring = get_measuring(span);
            rmp::encode::write_f64(writer, measuring)?;
            rmp::encode::write_u32(writer, span_type)?;
        }
    }

    Ok(())
}
