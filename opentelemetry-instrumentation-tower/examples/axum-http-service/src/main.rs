use axum::routing::{get, post, put, Router};
use bytes::Bytes;
use opentelemetry::global;
use opentelemetry_otlp::{
    WithExportConfig, {self},
};
use opentelemetry_sdk::metrics::PeriodicReader;
use opentelemetry_sdk::Resource;
use std::time::Duration;
use tower_otel_http_metrics;

const SERVICE_NAME: &str = "example-axum-http-service";
// Metric export interval should be less than or equal to 15s
// if the metrics may be converted to Prometheus metrics.
// Prometheus' query engine and compatible implementations
// require ~4 data points / interval for range queries,
// so queries ranging over 1m requre <= 15s scrape intervals.
// OTEL SDKS also respect the env var `OTEL_METRIC_EXPORT_INTERVAL` (no underscore prefix).
const _OTEL_METRIC_EXPORT_INTERVAL: Duration = Duration::from_secs(10);

fn init_otel_resource() -> Resource {
    Resource::builder().with_service_name(SERVICE_NAME).build()
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
    if rand_09::random_range(0..100) < PCT_SLOW_REQUESTS {
        let slow_request_secs = rand_09::random_range(0..=MAX_SLOW_REQUEST_SEC);
        tokio::time::sleep(Duration::from_secs(slow_request_secs)).await;
    };
    let body_size_multiple = rand_09::random_range(0..=MAX_BODY_SIZE_MULTIPLE);
    Bytes::from("hello world\n".repeat(body_size_multiple as usize))
}

#[tokio::main]
async fn main() {
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        // .with_endpoint("http://localhost:4317")  // default; leave out in favor of env var OTEL_EXPORTER_OTLP_ENDPOINT
        .build()
        .unwrap();

    let reader = PeriodicReader::builder(exporter)
        .with_interval(_OTEL_METRIC_EXPORT_INTERVAL)
        .build();

    let meter_provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(init_otel_resource())
        .build();

    global::set_meter_provider(meter_provider);
    // init our otel metrics middleware
    let global_meter = global::meter(SERVICE_NAME);
    let otel_metrics_service_layer = tower_otel_http_metrics::HTTPMetricsLayerBuilder::builder()
        .with_meter(global_meter)
        .build()
        .unwrap();

    let app = Router::new()
        .route("/", get(handle))
        .route("/", post(handle))
        .route("/", put(handle))
        .layer(otel_metrics_service_layer);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:5000").await.unwrap();
    let server = axum::serve(listener, app);

    if let Err(err) = server.await {
        eprintln!("server error: {}", err);
    }
}
