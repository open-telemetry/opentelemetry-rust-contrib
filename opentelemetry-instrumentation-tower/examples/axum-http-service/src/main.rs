use axum::routing::{get, post, put, Router};
use bytes::Bytes;
use opentelemetry::global;
use opentelemetry_instrumentation_tower::{
    HTTPLayerBuilder, ALTERNATE_HTTP_SERVER_DURATION_BOUNDS,
};
use opentelemetry_otlp::{MetricExporter, SpanExporter};
use opentelemetry_sdk::{
    metrics::{PeriodicReader, SdkMeterProvider},
    trace::SdkTracerProvider,
};
use std::time::Duration;

const SERVICE_NAME: &str = "example-axum-http-service";
// Metric export interval should be less than or equal to 15s
// if the metrics may be converted to Prometheus metrics.
// Prometheus' query engine and compatible implementations
// require ~4 data points / interval for range queries,
// so queries ranging over 1m requre <= 15s scrape intervals.
// OTEL SDKS also respect the env var `OTEL_METRIC_EXPORT_INTERVAL` (no underscore prefix).
const _OTEL_METRIC_EXPORT_INTERVAL: Duration = Duration::from_secs(10);

fn init_otel_resource() -> opentelemetry_sdk::Resource {
    opentelemetry_sdk::Resource::builder()
        .with_service_name(SERVICE_NAME)
        .build()
}

// PCT_SLOW_REQUESTS and MAX_SLOW_REQUEST_SEC are used to inject latency into some responses
// in order to utilize the higher request duration buckets in the request duration histogram.
// These values are chosen so that with the load-gen script's max 100 VUs, we get just enough
// slow requests to show up on the histograms without completely blocking up the server.
const PCT_SLOW_REQUESTS: u64 = 5;
const MAX_SLOW_REQUEST_SEC: u64 = 16;
// MAX_BODY_SIZE_MULTIPLE is used to demonstrate the `http.server.response.body.size` histogram
const MAX_BODY_SIZE_MULTIPLE: u64 = 16;

#[axum::debug_handler]
async fn handle() -> Bytes {
    if rand::random_range(0..100) < PCT_SLOW_REQUESTS {
        let slow_request_secs = rand::random_range(0..=MAX_SLOW_REQUEST_SEC);
        tokio::time::sleep(Duration::from_secs(slow_request_secs)).await;
    };
    let body_size_multiple = rand::random_range(0..=MAX_BODY_SIZE_MULTIPLE);
    Bytes::from("hello world\n".repeat(body_size_multiple as usize))
}

#[tokio::main]
async fn main() {
    {
        let exporter = MetricExporter::builder()
            .with_tonic()
            // .with_endpoint("http://localhost:4317")  // default; leave out in favor of env var OTEL_EXPORTER_OTLP_ENDPOINT
            .build()
            .unwrap();

        let reader = PeriodicReader::builder(exporter)
            .with_interval(_OTEL_METRIC_EXPORT_INTERVAL)
            .build();

        let provider = SdkMeterProvider::builder()
            .with_reader(reader)
            .with_resource(init_otel_resource())
            .build();

        global::set_meter_provider(provider);
    }

    {
        let exporter = SpanExporter::builder()
            .with_tonic()
            // .with_endpoint("http://localhost:4317")  // default; leave out in favor of env var OTEL_EXPORTER_OTLP_ENDPOINT
            .build()
            .unwrap();

        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .with_resource(init_otel_resource())
            .build();

        global::set_tracer_provider(provider);
    }

    let otel_service_layer = HTTPLayerBuilder::builder()
        .with_request_duration_bounds(Vec::from(ALTERNATE_HTTP_SERVER_DURATION_BOUNDS))
        .build()
        .unwrap();

    let app = Router::new()
        .route("/", get(handle))
        .route("/", post(handle))
        .route("/", put(handle))
        .layer(otel_service_layer);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:5000").await.unwrap();
    let server = axum::serve(listener, app);

    if let Err(err) = server.await {
        eprintln!("server error: {err}");
    }
}
