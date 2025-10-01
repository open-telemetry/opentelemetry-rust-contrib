use crate::{exporter::ModelConfig, DatadogTraceState};
use http::uri;
use opentelemetry::Value;
use opentelemetry_sdk::{
    trace::{self, SpanData},
    ExportError, Resource,
};
use std::fmt::Debug;
use url::ParseError;

use self::unified_tags::UnifiedTags;

use super::Mapping;

pub mod unified_tags;
mod v03;
mod v05;
mod v07;

// todo: we should follow the same mapping defined in https://github.com/DataDog/datadog-agent/blob/main/pkg/trace/api/otlp.go

// https://github.com/DataDog/dd-trace-js/blob/c89a35f7d27beb4a60165409376e170eacb194c5/packages/dd-trace/src/constants.js#L4
static SAMPLING_PRIORITY_KEY: &str = "_sampling_priority_v1";

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

// https://github.com/DataDog/datadog-agent/blob/ec96f3c24173ec66ba235bda7710504400d9a000/pkg/trace/traceutil/span.go#L20
static DD_MEASURED_KEY: &str = "_dd.measured";

fn get_measuring(span: &SpanData) -> f64 {
    if span.span_context.trace_state().measuring_enabled() {
        1.0
    } else {
        0.0
    }
}

/// Custom mapping between opentelemetry spans and datadog spans.
///
/// User can provide custom function to change the mapping. It currently supports customizing the following
/// fields in Datadog span protocol.
///
/// |field name|default value|
/// |---------------|-------------|
/// |service name| service name configuration from [`ModelConfig`]|
/// |name | opentelemetry instrumentation library name |
/// |resource| opentelemetry name|
///
/// The function takes a reference to [`SpanData`]() and a reference to [`ModelConfig`]() as parameters.
/// It should return a `&str` which will be used as the value for the field.
///
/// If no custom mapping is provided. Default mapping detailed above will be used.
///
/// For example,
/// ```no_run
/// use opentelemetry::global;
/// use opentelemetry_datadog::{ApiVersion, new_pipeline};
///
/// fn main() -> Result<(), opentelemetry_sdk::trace::TraceError> {
///     let provider = new_pipeline()
///         .with_service_name("my_app")
///         .with_api_version(ApiVersion::Version05)
///         // the custom mapping below will change the all spans' name to datadog spans
///         .with_name_mapping(|span, model_config|{"datadog spans"})
///         .with_agent_endpoint("http://localhost:8126")
///         .install_batch()?;
///     global::set_tracer_provider(provider.clone());
///     let tracer = global::tracer("opentelemetry-datadog-demo");
///
///     Ok(())
/// }
/// ```
pub type FieldMappingFn = dyn for<'a> Fn(&'a SpanData, &'a ModelConfig) -> &'a str + Send + Sync;

pub(crate) type FieldMapping = std::sync::Arc<FieldMappingFn>;

// Datadog uses some magic tags in their models. There is no recommended mapping defined in
// opentelemetry spec. Below is default mapping we gonna uses. Users can override it by providing
// their own implementations.
fn default_service_name_mapping<'a>(_span: &'a SpanData, config: &'a ModelConfig) -> &'a str {
    config.service_name.as_str()
}

fn default_name_mapping<'a>(span: &'a SpanData, _config: &'a ModelConfig) -> &'a str {
    span.instrumentation_scope.name()
}

fn default_resource_mapping<'a>(span: &'a SpanData, _config: &'a ModelConfig) -> &'a str {
    span.name.as_ref()
}

fn get_span_type(span: &SpanData) -> Option<&Value> {
    for kv in &span.attributes {
        if kv.key.as_str() == "span.type" {
            return Some(&kv.value);
        }
    }

    None
}

/// Wrap type for errors from opentelemetry datadog exporter
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Message pack error
    #[error("message pack error")]
    MessagePackError,
    /// No http client founded. User should provide one or enable features
    #[error("http client must be set, users can enable reqwest or surf feature to use http client implementation within create")]
    NoHttpClient,
    /// Http requests failed with following errors
    #[error(transparent)]
    RequestError(#[from] http::Error),
    /// The Uri was invalid
    #[error("invalid url {0}")]
    InvalidUri(String),
    /// Other errors
    #[error("{0}")]
    Other(String),
}

impl ExportError for Error {
    fn exporter_name(&self) -> &'static str {
        "datadog"
    }
}

impl From<rmp::encode::ValueWriteError> for Error {
    fn from(_: rmp::encode::ValueWriteError) -> Self {
        Self::MessagePackError
    }
}

impl From<url::ParseError> for Error {
    fn from(err: ParseError) -> Self {
        Self::InvalidUri(err.to_string())
    }
}

impl From<uri::InvalidUri> for Error {
    fn from(err: uri::InvalidUri) -> Self {
        Self::InvalidUri(err.to_string())
    }
}

/// Version of datadog trace ingestion API
#[derive(Debug, Copy, Clone)]
#[non_exhaustive]
pub enum ApiVersion {
    /// Version 0.3
    Version03,
    /// Version 0.5 - requires datadog-agent v7.22.0 or above
    Version05,
    /// Version 0.7
    Version07,
}

impl ApiVersion {
    pub(crate) fn path(self) -> &'static str {
        match self {
            ApiVersion::Version03 => "/v0.3/traces",
            ApiVersion::Version05 => "/v0.5/traces",
            ApiVersion::Version07 => "/v0.7/traces",
        }
    }

    pub(crate) fn content_type(self) -> &'static str {
        match self {
            ApiVersion::Version03 => "application/msgpack",
            ApiVersion::Version05 => "application/msgpack",
            ApiVersion::Version07 => "application/msgpack",
        }
    }

    pub(crate) fn encode(
        self,
        model_config: &ModelConfig,
        traces: Vec<&[trace::SpanData]>,
        mapping: &Mapping,
        unified_tags: &UnifiedTags,
        resource: Option<&Resource>,
    ) -> Result<Vec<u8>, Error> {
        match self {
            Self::Version03 => v03::encode(
                model_config,
                traces,
                |span, config| match &mapping.service_name {
                    Some(f) => f(span, config),
                    None => default_service_name_mapping(span, config),
                },
                |span, config| match &mapping.name {
                    Some(f) => f(span, config),
                    None => default_name_mapping(span, config),
                },
                |span, config| match &mapping.resource {
                    Some(f) => f(span, config),
                    None => default_resource_mapping(span, config),
                },
                resource,
            ),
            Self::Version05 => v05::encode(
                model_config,
                traces,
                |span, config| match &mapping.service_name {
                    Some(f) => f(span, config),
                    None => default_service_name_mapping(span, config),
                },
                |span, config| match &mapping.name {
                    Some(f) => f(span, config),
                    None => default_name_mapping(span, config),
                },
                |span, config| match &mapping.resource {
                    Some(f) => f(span, config),
                    None => default_resource_mapping(span, config),
                },
                unified_tags,
                resource,
            ),
            Self::Version07 => v07::encode(
                model_config,
                traces,
                |span, config| match &mapping.service_name {
                    Some(f) => f(span, config),
                    None => default_service_name_mapping(span, config),
                },
                |span, config| match &mapping.name {
                    Some(f) => f(span, config),
                    None => default_name_mapping(span, config),
                },
                |span, config| match &mapping.resource {
                    Some(f) => f(span, config),
                    None => default_resource_mapping(span, config),
                },
                unified_tags,
                resource,
            ),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use opentelemetry::trace::Event;
    use opentelemetry::InstrumentationScope;
    use opentelemetry::{
        trace::{SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState},
        KeyValue,
    };
    use opentelemetry_sdk::{
        self,
        trace::{SpanEvents, SpanLinks},
    };
    use std::time::{Duration, SystemTime};

    fn get_traces() -> Vec<Vec<trace::SpanData>> {
        vec![vec![get_span(7, 1, 99)]]
    }

    fn get_traces_with_events() -> Vec<Vec<trace::SpanData>> {
        let event = Event::new(
            "myevent",
            SystemTime::UNIX_EPOCH
                .checked_add(Duration::from_secs(5))
                .unwrap(),
            vec![
                KeyValue::new("mykey", 1),
                KeyValue::new(
                    "myarray",
                    Value::Array(opentelemetry::Array::String(vec![
                        "myvalue1".into(),
                        "myvalue2".into(),
                    ])),
                ),
                KeyValue::new("mybool", true),
                KeyValue::new("myint", 2.5),
                KeyValue::new("myboolfalse", false),
            ],
            0,
        );
        let mut events = SpanEvents::default();
        events.events.push(event);

        vec![vec![get_span_with_events(7, 1, 99, events)]]
    }

    pub(crate) fn get_span(trace_id: u128, parent_span_id: u64, span_id: u64) -> trace::SpanData {
        get_span_with_events(trace_id, parent_span_id, span_id, SpanEvents::default())
    }

    pub(crate) fn get_span_with_events(
        trace_id: u128,
        parent_span_id: u64,
        span_id: u64,
        events: SpanEvents,
    ) -> trace::SpanData {
        let span_context = SpanContext::new(
            TraceId::from(trace_id),
            SpanId::from(span_id),
            TraceFlags::default(),
            false,
            TraceState::default(),
        );

        let start_time = SystemTime::UNIX_EPOCH;
        let end_time = start_time.checked_add(Duration::from_secs(1)).unwrap();

        let attributes = vec![KeyValue::new("span.type", "web")];
        let links = SpanLinks::default();
        let instrumentation_scope = InstrumentationScope::builder("component").build();

        trace::SpanData {
            span_context,
            parent_span_id: SpanId::from(parent_span_id),
            parent_span_is_remote: false,
            span_kind: SpanKind::Client,
            name: "resource".into(),
            start_time,
            end_time,
            attributes,
            dropped_attributes_count: 0,
            events,
            links,
            status: Status::Ok,
            instrumentation_scope,
        }
    }

    #[test]
    fn test_encode_v03() -> Result<(), Box<dyn std::error::Error>> {
        let traces = get_traces();
        let model_config = ModelConfig {
            service_name: "service_name".to_string(),
            ..Default::default()
        };
        let resource = Resource::builder_empty()
            .with_attribute(KeyValue::new("host.name", "test"))
            .build();
        let encoded = STANDARD.encode(ApiVersion::Version03.encode(
            &model_config,
            traces.iter().map(|x| &x[..]).collect(),
            &Mapping::empty(),
            &UnifiedTags::new(),
            Some(&resource),
        )?);

        assert_eq!(encoded.as_str(), "kZGMpHR5cGWjd2Vip3NlcnZpY2Wsc2VydmljZV9uYW1lpG5hbWWpY29tcG9uZW\
        50qHJlc291cmNlqHJlc291cmNlqHRyYWNlX2lkzwAAAAAAAAAHp3NwYW5faWTPAAAAAAAAAGOpcGFyZW50X2lkzwAAAA\
        AAAAABpXN0YXJ00wAAAAAAAAAAqGR1cmF0aW9u0wAAAAA7msoApWVycm9y0gAAAACkbWV0YYKpaG9zdC5uYW1lpHRlc3\
        Spc3Bhbi50eXBlo3dlYqdtZXRyaWNzgbVfc2FtcGxpbmdfcHJpb3JpdHlfdjHLAAAAAAAAAAA=");

        Ok(())
    }

    #[test]
    fn test_encode_v05() -> Result<(), Box<dyn std::error::Error>> {
        let traces = get_traces();
        let model_config = ModelConfig {
            service_name: "service_name".to_string(),
            ..Default::default()
        };
        let resource = Resource::builder()
            .with_attribute(KeyValue::new("host.name", "test"))
            .build();

        let mut unified_tags = UnifiedTags::new();
        unified_tags.set_env(Some(String::from("test-env")));
        unified_tags.set_version(Some(String::from("test-version")));
        unified_tags.set_service(Some(String::from("test-service")));

        let _encoded = STANDARD.encode(ApiVersion::Version05.encode(
            &model_config,
            traces.iter().map(|x| &x[..]).collect(),
            &Mapping::empty(),
            &unified_tags,
            Some(&resource),
        )?);

        // TODO: Need someone to generate the expected result or instructions to do so.
        // assert_eq!(encoded.as_str(), "kp6jd2VirHNlcnZpY2VfbmFtZaljb21wb25lbnSocmVzb3VyY2WpaG9zdC5uYW\
        // 1lpHRlc3Snc2VydmljZax0ZXN0LXNlcnZpY2WjZW52qHRlc3QtZW52p3ZlcnNpb26sdGVzdC12ZXJzaW9uqXNwYW4udH\
        // lwZbVfc2FtcGxpbmdfcHJpb3JpdHlfdjGRkZzOAAAAAc4AAAACzgAAAAPPAAAAAAAAAAfPAAAAAAAAAGPPAAAAAAAAAA\
        // HTAAAAAAAAAADTAAAAADuaygDSAAAAAIXOAAAABM4AAAAFzgAAAAbOAAAAB84AAAAIzgAAAAnOAAAACs4AAAALzgAAAA\
        // zOAAAAAIHOAAAADcsAAAAAAAAAAM4AAAAA");

        Ok(())
    }

    #[test]
    fn test_encode_v07() {
        let traces = get_traces_with_events();
        let model_config = ModelConfig {
            service_name: "service_name".to_string(),
            ..Default::default()
        };

        // we use an empty builder with a single attribute because the attributes are in a hashmap
        // which causes the order to change every test
        let resource = Resource::builder_empty()
            .with_attribute(KeyValue::new("host.name", "test"))
            .build();

        let mut unified_tags = UnifiedTags::new();
        unified_tags.set_env(Some(String::from("test-env")));
        unified_tags.set_version(Some(String::from("test-version")));
        unified_tags.set_service(Some(String::from("test-service")));

        let encoded = STANDARD.encode(
            ApiVersion::Version07
                .encode(
                    &model_config,
                    traces.iter().map(|x| &x[..]).collect(),
                    &Mapping::empty(),
                    &unified_tags,
                    Some(&resource),
                )
                .unwrap(),
        );

        // A very nice way to check the encoded values is to use
        // https://github.com/DataDog/dd-apm-test-agent
        // Which is a test http server that receives and validates sent traces
        let expected = "ha1sYW5ndWFnZV9uYW1lpHJ1c3SmY2h1bmtzkYOocHJpb3JpdHnSAAAAAaZvcmlnaW6gpXNwY\
        W5zkY6kbmFtZaljb21wb25lbnSnc3Bhbl9pZM8AAAAAAAAAY6h0cmFjZV9pZM8AAAAAAAAAB6VzdGFydNMAAAAAAAAAAKhk\
        dXJhdGlvbtMAAAAAO5rKAKlwYXJlbnRfaWTPAAAAAAAAAAGnc2VydmljZaxzZXJ2aWNlX25hbWWocmVzb3VyY2WocmVzb3V\
        yY2WkdHlwZaN3ZWKlZXJyb3LSAAAAAKRtZXRhgqlob3N0Lm5hbWWkdGVzdKlzcGFuLnR5cGWjd2Vip21ldHJpY3OCtV9zYW\
        1wbGluZ19wcmlvcml0eV92Mcs/8AAAAAAAAKxfZGQubWVhc3VyZWTLAAAAAAAAAACqc3Bhbl9saW5rc5Crc3Bhbl9ldmVud\
        HORg6RuYW1lp215ZXZlbnSudGltZV91bml4X25hbm/TAAAAASoF8gCqYXR0cmlidXRlc4WlbXlrZXmCpHR5cGXSAAAAAqlp\
        bnRfdmFsdWXTAAAAAAAAAAGnbXlhcnJheYKkdHlwZdIAAAAEq2FycmF5X3ZhbHVlkoKkdHlwZQCsc3RyaW5nX3ZhbHVlqG1\
        5dmFsdWUxgqR0eXBlAKxzdHJpbmdfdmFsdWWobXl2YWx1ZTKmbXlib29sgqR0eXBl0gAAAAGqYm9vbF92YWx1ZcOlbXlpbn\
        SCpHR5cGXSAAAAA6xkb3VibGVfdmFsdWXLQAQAAAAAAACrbXlib29sZmFsc2WCpHR5cGXSAAAAAapib29sX3ZhbHVlwqR0Y\
        Wdzg6dzZXJ2aWNlrHRlc3Qtc2VydmljZad2ZXJzaW9urHRlc3QtdmVyc2lvbqNlbnaodGVzdC1lbnajZW52qHRlc3QtZW52\
        q2FwcF92ZXJzaW9urHRlc3QtdmVyc2lvbg==";
        assert_eq!(encoded.as_str(), expected);

        // change to a different resource and make sure the encoded value changes and that we actually encode stuff
        let other_resource = Resource::builder_empty()
            .with_attribute(KeyValue::new("host.name", "thisissometingelse"))
            .build();

        let encoded = STANDARD.encode(
            ApiVersion::Version07
                .encode(
                    &model_config,
                    traces.iter().map(|x| &x[..]).collect(),
                    &Mapping::empty(),
                    &unified_tags,
                    Some(&other_resource),
                )
                .unwrap(),
        );

        assert_ne!(encoded.as_str(), expected);
    }
}
