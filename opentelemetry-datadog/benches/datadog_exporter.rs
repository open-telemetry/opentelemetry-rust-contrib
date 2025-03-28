use std::time::{Duration, SystemTime};

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use http::Request;
use opentelemetry::{
    trace::{SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState},
    Array, InstrumentationScope, KeyValue, Value,
};
use opentelemetry_datadog::{new_pipeline, ApiVersion};
use opentelemetry_http::HttpClient;
use opentelemetry_sdk::{
    trace::{SpanData, SpanExporter},
    trace::{SpanEvents, SpanLinks},
};
use rand::seq::{IndexedRandom, SliceRandom};
use rand::{rng, rngs::ThreadRng, RngCore};

#[derive(Debug)]
struct DummyClient;

#[async_trait::async_trait]
impl HttpClient for DummyClient {
    async fn send(
        &self,
        _request: Request<Vec<u8>>,
    ) -> Result<http::Response<bytes::Bytes>, opentelemetry_http::HttpError> {
        Ok(http::Response::new("dummy response".into()))
    }
    async fn send_bytes(
        &self,
        request: Request<Bytes>,
    ) -> Result<http::Response<Bytes>, opentelemetry_http::HttpError> {
        Ok(http::Response::builder()
            .status(200)
            .body(request.into_body())
            .unwrap())
    }
}

fn get_http_method(rng: &mut ThreadRng) -> String {
    const HTTP_METHODS: [&str; 4] = ["GET", "POST", "PUT", "DELETE"];
    HTTP_METHODS.choose(rng).unwrap().to_string()
}

fn get_http_route(rng: &mut ThreadRng) -> String {
    const HTTP_ROUTES: [&str; 4] = [
        "/v1/user/{user_id}",
        "/v1/student/{student_id}",
        "/v2/family/{family_id}",
        "/v3/awesome/endpoint",
    ];
    HTTP_ROUTES.choose(rng).unwrap().to_string()
}

fn get_http_target(rng: &mut ThreadRng) -> String {
    let id = rng.next_u32();
    let targets = [
        format!("/v1/user/{id}"),
        format!("/v1/student/{id}"),
        format!("/v2/family/{id}"),
        "/v3/awesome/endpoint".to_string(),
    ];
    targets.choose(rng).unwrap().to_string()
}

fn get_http_scheme(rng: &mut ThreadRng) -> String {
    const HTTP_SCHEME: [&str; 2] = ["http", "https"];
    HTTP_SCHEME.choose(rng).unwrap().to_string()
}

fn get_http_user_agent(rng: &mut ThreadRng) -> String {
    const HTTP_USER_AGENT : [&str; 7] = [
        "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/114.0.0.0 Mobile Safari/537.36",
        "Mozilla/5.0 (Linux; Android 13; SM-S901B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/112.0.0.0 Mobile Safari/537.36",
        "Mozilla/5.0 (iPhone14,6; U; CPU iPhone OS 15_4 like Mac OS X) AppleWebKit/602.1.50 (KHTML, like Gecko) Version/10.0 Mobile/19E241 Safari/602.1",
        "Mozilla/5.0 (iPhone14,3; U; CPU iPhone OS 15_0 like Mac OS X) AppleWebKit/602.1.50 (KHTML, like Gecko) Version/10.0 Mobile/19A346 Safari/602.1",
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/42.0.2311.135 Safari/537.36 Edge/12.246",
        "Mozilla/5.0 (X11; CrOS x86_64 8172.45.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/51.0.2704.64 Safari/537.36",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_11_2) AppleWebKit/601.3.9 (KHTML, like Gecko) Version/9.0.2 Safari/601.3.9",
    ];
    HTTP_USER_AGENT.choose(rng).unwrap().to_string()
}

fn get_http_flavor(rng: &mut ThreadRng) -> String {
    const HTTP_FLAVOR: [&str; 3] = ["1.1", "2", "3"];
    HTTP_FLAVOR.choose(rng).unwrap().to_string()
}

fn get_http_client_id(rng: &mut ThreadRng) -> String {
    rng.next_u32()
        .to_be_bytes()
        .map(|x| x.to_string())
        .join(".")
}

fn get_int_value(rng: &mut ThreadRng) -> i64 {
    rng.next_u32() as i64
}

fn get_float_value(rng: &mut ThreadRng) -> f64 {
    rng.next_u32() as f64
}

fn get_boolean_value(rng: &mut ThreadRng) -> bool {
    rng.next_u32() % 2 == 0
}

fn get_array_of_ints(rng: &mut ThreadRng) -> Value {
    let len = rng.next_u32() % 8;
    Value::Array(Array::I64((0..len).map(|_| get_int_value(rng)).collect()))
}

fn get_array_of_floats(rng: &mut ThreadRng) -> Value {
    let len = rng.next_u32() % 8;
    Value::Array(Array::F64((0..len).map(|_| get_float_value(rng)).collect()))
}

fn get_array_of_booleans(rng: &mut ThreadRng) -> Value {
    let len = rng.next_u32() % 8;
    Value::Array(Array::Bool(
        (0..len).map(|_| get_boolean_value(rng)).collect(),
    ))
}

fn get_span(trace_id: u128, parent_span_id: u64, span_id: u64, rng: &mut ThreadRng) -> SpanData {
    let span_context = SpanContext::new(
        TraceId::from_u128(trace_id),
        SpanId::from_u64(span_id),
        TraceFlags::default(),
        false,
        TraceState::default(),
    );

    let start_time = SystemTime::UNIX_EPOCH;
    let end_time = start_time.checked_add(Duration::from_secs(1)).unwrap();

    let attributes = vec![
        KeyValue::new("span.type", "web"),
        KeyValue::new("http.method", get_http_method(rng)),
        KeyValue::new("http.route", get_http_route(rng)),
        KeyValue::new("http.scheme", get_http_scheme(rng)),
        KeyValue::new("http.host", "my.awesome.server"),
        KeyValue::new("http.client_id", get_http_client_id(rng)),
        KeyValue::new("http.flavor", get_http_flavor(rng)),
        KeyValue::new("http.target", get_http_target(rng)),
        KeyValue::new("http.user_agent", get_http_user_agent(rng)),
        KeyValue::new("property_int_1", get_int_value(rng)),
        KeyValue::new("property_int_2", get_int_value(rng)),
        KeyValue::new("property_int_3", get_int_value(rng)),
        KeyValue::new("property_float_1", get_float_value(rng)),
        KeyValue::new("property_float_2", get_float_value(rng)),
        KeyValue::new("property_float_3", get_float_value(rng)),
        KeyValue::new("property_boolean_1", get_boolean_value(rng)),
        KeyValue::new("property_boolean_2", get_boolean_value(rng)),
        KeyValue::new("property_boolean_3", get_boolean_value(rng)),
        KeyValue::new("property_boolean_array", get_array_of_booleans(rng)),
        KeyValue::new("property_int_array", get_array_of_ints(rng)),
        KeyValue::new("property_float_array", get_array_of_floats(rng)),
    ];
    let events = SpanEvents::default();
    let links = SpanLinks::default();
    let instrumentation_scope = InstrumentationScope::builder("component").build();

    SpanData {
        span_context,
        parent_span_id: SpanId::from_u64(parent_span_id),
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

fn generate_traces(number_of_traces: usize, spans_per_trace: usize) -> Vec<SpanData> {
    let mut rng = rng();

    let mut result: Vec<SpanData> = (0..number_of_traces)
        .flat_map(|trace_id| {
            let id = &trace_id;
            (0..spans_per_trace)
                .map(|span_id| get_span(*id as u128, span_id as u64, span_id as u64, &mut rng))
                .collect::<Vec<_>>()
        })
        .collect();

    result.shuffle(&mut rng);

    result
}

fn criterion_benchmark(c: &mut Criterion) {
    let exporter = new_pipeline()
        .with_service_name("trace-demo")
        .with_api_version(ApiVersion::Version05)
        .with_http_client(DummyClient)
        .build_exporter()
        .unwrap();

    let patterns: [(usize, usize); 5] = [(128, 4), (256, 4), (512, 4), (512, 2), (512, 1)];

    for (number_of_traces, spans_per_trace) in patterns {
        let data = generate_traces(number_of_traces, spans_per_trace);
        let data_ref = &data;

        c.bench_function(
            format!("export {number_of_traces} traces with {spans_per_trace} spans").as_str(),
            |b| b.iter(|| exporter.export(black_box(data_ref.clone()))),
        );
    }
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
