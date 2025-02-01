use crate::exporter::model::{Error, SAMPLING_PRIORITY_KEY};
use crate::exporter::ModelConfig;
use opentelemetry::trace::Status;
use opentelemetry_sdk::export::trace::SpanData;
use opentelemetry_sdk::Resource;
use std::time::SystemTime;

pub(crate) fn encode<S, N, R, W: std::io::Write>(
    writer: &mut W,
    model_config: &ModelConfig,
    traces: Vec<&[SpanData]>,
    get_service_name: S,
    get_name: N,
    get_resource: R,
    resource: Option<&Resource>,
) -> Result<(), Error>
where
    for<'a> S: Fn(&'a SpanData, &'a ModelConfig) -> &'a str,
    for<'a> N: Fn(&'a SpanData, &'a ModelConfig) -> &'a str,
    for<'a> R: Fn(&'a SpanData, &'a ModelConfig) -> &'a str,
{
    rmp::encode::write_array_len(writer, traces.len() as u32)?;

    for trace in traces.into_iter() {
        rmp::encode::write_array_len(writer, trace.len() as u32)?;

        for span in trace {
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

            let mut span_type_found = false;
            for kv in &span.attributes {
                if kv.key.as_str() == "span.type" {
                    span_type_found = true;
                    rmp::encode::write_map_len(writer, 12)?;
                    rmp::encode::write_str(writer, "type")?;
                    rmp::encode::write_str(writer, kv.value.as_str().as_ref())?;
                    break;
                }
            }

            if !span_type_found {
                rmp::encode::write_map_len(writer, 11)?;
            }

            // Datadog span name is OpenTelemetry component name - see module docs for more information
            rmp::encode::write_str(writer, "service")?;
            rmp::encode::write_str(writer, get_service_name(span, model_config))?;

            rmp::encode::write_str(writer, "name")?;
            rmp::encode::write_str(writer, get_name(span, model_config))?;

            rmp::encode::write_str(writer, "resource")?;
            rmp::encode::write_str(writer, get_resource(span, model_config))?;

            rmp::encode::write_str(writer, "trace_id")?;
            rmp::encode::write_u64(
                writer,
                u128::from_be_bytes(span.span_context.trace_id().to_bytes()) as u64,
            )?;

            rmp::encode::write_str(writer, "span_id")?;
            rmp::encode::write_u64(
                writer,
                u64::from_be_bytes(span.span_context.span_id().to_bytes()),
            )?;

            rmp::encode::write_str(writer, "parent_id")?;
            rmp::encode::write_u64(writer, u64::from_be_bytes(span.parent_span_id.to_bytes()))?;

            rmp::encode::write_str(writer, "start")?;
            rmp::encode::write_i64(writer, start)?;

            rmp::encode::write_str(writer, "duration")?;
            rmp::encode::write_i64(writer, duration)?;

            rmp::encode::write_str(writer, "error")?;
            rmp::encode::write_i32(
                writer,
                match span.status {
                    Status::Error { .. } => 1,
                    _ => 0,
                },
            )?;

            rmp::encode::write_str(writer, "meta")?;
            rmp::encode::write_map_len(
                writer,
                (span.attributes.len() + resource.map(|r| r.len()).unwrap_or(0)) as u32,
            )?;
            if let Some(resource) = resource {
                for (key, value) in resource.iter() {
                    rmp::encode::write_str(writer, key.as_str())?;
                    rmp::encode::write_str(writer, value.as_str().as_ref())?;
                }
            }
            for kv in span.attributes.iter() {
                rmp::encode::write_str(writer, kv.key.as_str())?;
                rmp::encode::write_str(writer, kv.value.as_str().as_ref())?;
            }

            rmp::encode::write_str(writer, "metrics")?;
            rmp::encode::write_map_len(writer, 1)?;
            rmp::encode::write_str(writer, SAMPLING_PRIORITY_KEY)?;
            rmp::encode::write_f64(
                writer,
                if span.span_context.is_sampled() {
                    1.0
                } else {
                    0.0
                },
            )?;
        }
    }

    Ok(())
}
