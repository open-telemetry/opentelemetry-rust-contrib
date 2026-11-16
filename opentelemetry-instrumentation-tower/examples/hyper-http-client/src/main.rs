use http_body_util::Empty;
use hyper::body::Bytes;
use hyper::Request;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use opentelemetry::global;
use opentelemetry_instrumentation_tower::http::client::LayerBuilder;
use opentelemetry_otlp::{MetricExporter, SpanExporter};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::SdkTracerProvider;
use std::sync::OnceLock;
use tower::{Service, ServiceBuilder};

const SERVICE_NAME: &str = "example-hyper-http-client";

fn get_resource() -> opentelemetry_sdk::Resource {
    static RESOURCE: OnceLock<opentelemetry_sdk::Resource> = OnceLock::new();
    RESOURCE
        .get_or_init(|| {
            opentelemetry_sdk::Resource::builder()
                .with_service_name(SERVICE_NAME)
                .build()
        })
        .clone()
}

#[tokio::main]
async fn main() {
    let metric_exporter = MetricExporter::builder().with_tonic().build().unwrap();
    let reader = PeriodicReader::builder(metric_exporter).build();
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(get_resource())
        .build();
    global::set_meter_provider(meter_provider.clone());

    let span_exporter = SpanExporter::builder().with_tonic().build().unwrap();
    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(span_exporter)
        .with_resource(get_resource())
        .build();
    global::set_tracer_provider(tracer_provider.clone());

    let connector = hyper_util::client::legacy::connect::HttpConnector::new();
    let hyper_client = Client::builder(TokioExecutor::new()).build::<_, Empty<Bytes>>(connector);

    let otel_layer = LayerBuilder::builder().build().unwrap();

    let mut client = ServiceBuilder::new()
        .layer(otel_layer)
        .service(hyper_client);

    let target_url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://localhost:5000/".to_string());

    let req = Request::builder()
        .method("GET")
        .uri(&target_url)
        .body(Empty::<Bytes>::new())
        .unwrap();

    let res = client.call(req).await.unwrap();
    println!("response: {}", res.status());

    let _ = tracer_provider.shutdown();
    let _ = meter_provider.shutdown();
}
