# Tower OTEL Metrics Middleware

OpenTelemetry Metrics Middleware for Tower-compatible Rust HTTP servers.

## Examples

See `examples` directory in repo for runnable code and supporting config files.
Attempts are made to keep the code here synced, but it will not be perfect.

OTEL libraries in particular are sensitive to minor version changes at this point,
so the examples may only work with the OTEL crate versions pinned in `examples`.

### Axum Server

Adding OpenTelementry HTTP Server metrics using the [`Axum`](https://docs.rs/axum/latest/axum) framework
over a Tower-compatible [`Hyper`](https://docs.rs/hyper/latest/hyper) Service:

```rust
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
        .with_endpoint("http://localhost:4317")
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
```

### Hyper Server

Adding OpenTelementry HTTP Server metrics to a bare-bones Tower-compatible Service
using [`Hyper`](https://docs.rs/crate/hyper/latest):

```rust
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::{Request, Response};
use opentelemetry::global;
use opentelemetry_otlp::{
    WithExportConfig, {self},
};
use opentelemetry_sdk::metrics::PeriodicReader;
use opentelemetry_sdk::Resource;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_otel_http_metrics;

const SERVICE_NAME: &str = "example-hyper-http-service";
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

async fn handle(_req: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    if rand_09::random_range(0..100) < PCT_SLOW_REQUESTS {
        let slow_request_secs = rand_09::random_range(0..=MAX_SLOW_REQUEST_SEC);
        tokio::time::sleep(Duration::from_secs(slow_request_secs)).await;
    };
    let body_size_multiple = rand_09::random_range(0..=MAX_BODY_SIZE_MULTIPLE);
    let body = Bytes::from("hello world\n".repeat(body_size_multiple as usize));
    Ok(Response::new(Full::new(body)))
}

#[tokio::main]
async fn main() {
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint("http://localhost:4317")
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

    let tower_service = ServiceBuilder::new()
        .layer(otel_metrics_service_layer)
        .service_fn(handle);
    let hyper_service = hyper_util::service::TowerToHyperService::new(tower_service);

    let addr = SocketAddr::from(([0, 0, 0, 0], 5000));
    let listener = TcpListener::bind(addr).await.unwrap();

    loop {
        let (stream, _) = listener.accept().await.unwrap();

        let io = hyper_util::rt::TokioIo::new(stream);
        let service_clone = hyper_service.clone();

        tokio::task::spawn(async move {
            if let Err(err) =
                hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
                    .serve_connection(io, service_clone)
                    .await
            {
                eprintln!("server error: {}", err);
            }
        });
    }
}
```

Created by Franco Posa (franco @ [francoposa.io](https://francoposa.io))
