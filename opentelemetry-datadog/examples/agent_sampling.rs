use opentelemetry::{
    global::{self, shutdown_tracer_provider},
    trace::{SamplingResult, Span, TraceContextExt, Tracer},
    Key,
};
use opentelemetry_datadog::{new_pipeline, ApiVersion, DatadogTraceStateBuilder};
use opentelemetry_sdk::trace::{self, RandomIdGenerator, ShouldSample};
use std::thread;
use std::time::Duration;

fn bar() {
    let tracer = global::tracer("component-bar");
    let mut span = tracer.start("bar");
    span.set_attribute(Key::new("span.type").string("sql"));
    span.set_attribute(Key::new("sql.query").string("SELECT * FROM table"));
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
            .map(|parent_context|
                parent_context.span().span_context().trace_state().clone() // inherit sample decision from parent span
            )
            .unwrap_or_else(|| DatadogTraceStateBuilder::default()
                .with_priority_sampling(true) // always sample root span(span without remote or local parent)
                .with_measuring(true) // datadog-agent will create metric for this span for APM
                .build()
            );

        SamplingResult {
            decision: opentelemetry::trace::SamplingDecision::RecordAndSample, // send all spans to datadog-agent
            attributes: vec![],
            trace_state,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let tracer = new_pipeline()
        .with_service_name("agent-sampling-demo")
        .with_api_version(ApiVersion::Version05)
        .with_trace_config(
            trace::config()
                .with_sampler(AgentBasedSampler)
                .with_id_generator(RandomIdGenerator::default())
        )
        .install_simple()?;

    tracer.in_span("foo", |cx| {
        let span = cx.span();
        span.set_attribute(Key::new("span.type").string("web"));
        span.set_attribute(Key::new("http.url").string("http://localhost:8080/foo"));
        span.set_attribute(Key::new("http.method").string("GET"));
        span.set_attribute(Key::new("http.status_code").i64(200));

        thread::sleep(Duration::from_millis(6));
        bar();
        thread::sleep(Duration::from_millis(6));
    });

    shutdown_tracer_provider();

    Ok(())
}
