use opentelemetry::{
    global,
    trace::{SamplingResult, Span, TraceContextExt, Tracer, TracerProvider},
    InstrumentationScope, Key, KeyValue, Value,
};
use opentelemetry_datadog::{new_pipeline, ApiVersion, DatadogTraceStateBuilder};
use opentelemetry_sdk::trace::{self, RandomIdGenerator, ShouldSample};
use opentelemetry_semantic_conventions as semcov;
use std::thread;
use std::time::Duration;

fn bar() {
    let tracer = global::tracer("component-bar");
    let mut span = tracer.start("bar");
    span.set_attribute(KeyValue::new(
        Key::new("span.type"),
        Value::String("sql".into()),
    ));
    span.set_attribute(KeyValue::new(
        Key::new("sql.query"),
        Value::String("SELECT * FROM table".into()),
    ));
    thread::sleep(Duration::from_millis(6));
    span.end()
}

#[derive(Debug, Clone)]
struct AgentBasedSampler;

impl ShouldSample for AgentBasedSampler {
    fn should_sample(
        &self,
        parent_context: Option<&opentelemetry::Context>,
        _trace_id: opentelemetry::trace::TraceId,
        _name: &str,
        _span_kind: &opentelemetry::trace::SpanKind,
        _attributes: &[opentelemetry::KeyValue],
        _links: &[opentelemetry::trace::Link],
    ) -> opentelemetry::trace::SamplingResult {
        let trace_state = parent_context
            .map(
                |parent_context| parent_context.span().span_context().trace_state().clone(), // inherit sample decision from parent span
            )
            .unwrap_or_else(|| {
                DatadogTraceStateBuilder::default()
                    .with_priority_sampling(true) // always sample root span(span without remote or local parent)
                    .with_measuring(true) // datadog-agent will create metric for this span for APM
                    .build()
            });

        SamplingResult {
            decision: opentelemetry::trace::SamplingDecision::RecordAndSample, // send all spans to datadog-agent
            attributes: vec![],
            trace_state,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let mut config = trace::Config::default();
    config.sampler = Box::new(AgentBasedSampler);
    config.id_generator = Box::new(RandomIdGenerator::default());

    let provider = new_pipeline()
        .with_service_name("agent-sampling-demo")
        .with_api_version(ApiVersion::Version05)
        .with_trace_config(config)
        .install_simple()?;
    global::set_tracer_provider(provider.clone());
    let scope = InstrumentationScope::builder("opentelemetry-datadog-demo")
        .with_version(env!("CARGO_PKG_VERSION"))
        .with_schema_url(semcov::SCHEMA_URL)
        .with_attributes(None)
        .build();
    let tracer = provider.tracer_with_scope(scope);

    tracer.in_span("foo", |cx| {
        let span = cx.span();
        span.set_attribute(KeyValue::new(
            Key::new("span.type"),
            Value::String("web".into()),
        ));
        span.set_attribute(KeyValue::new(
            Key::new("http.url"),
            Value::String("http://localhost:8080/foo".into()),
        ));
        span.set_attribute(KeyValue::new(
            Key::new("http.method"),
            Value::String("GET".into()),
        ));
        span.set_attribute(KeyValue::new(Key::new("http.status_code"), Value::I64(200)));

        thread::sleep(Duration::from_millis(6));
        bar();
        thread::sleep(Duration::from_millis(6));
    });

    provider.shutdown()?;

    Ok(())
}
