use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use hyper::Request;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use opentelemetry::global;
use opentelemetry_instrumentation_tower::http::client::LayerBuilder;
use opentelemetry_otlp::{MetricExporter, SpanExporter};
use opentelemetry_sdk::metrics::Aggregation::ExplicitBucketHistogram;
use opentelemetry_sdk::metrics::{Instrument, PeriodicReader, SdkMeterProvider, Stream};
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_semantic_conventions as semconv;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::select;
use tower::{Service, ServiceBuilder};

const SERVICE_NAME: &str = "example-hyper-http-client";
const _OTEL_METRIC_EXPORT_INTERVAL: Duration = Duration::from_secs(10);

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

const HISTOGRAM_BUCKET_BOUNDS: [f64; 14] = [
    0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0,
];

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("ctrl-c handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(unix)]
    select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    #[cfg(not(unix))]
    ctrl_c.await;
}

#[tokio::main]
async fn main() {
    {
        let exporter = MetricExporter::builder()
            .with_tonic()
            .build()
            .unwrap();

        let reader = PeriodicReader::builder(exporter)
            .with_interval(_OTEL_METRIC_EXPORT_INTERVAL)
            .build();

        let http_client_request_duration_view = |i: &Instrument| {
            if i.name() == semconv::metric::HTTP_CLIENT_REQUEST_DURATION {
                Stream::builder()
                    .with_aggregation(ExplicitBucketHistogram {
                        boundaries: Vec::from(HISTOGRAM_BUCKET_BOUNDS),
                        record_min_max: true,
                    })
                    .build()
                    .ok()
            } else {
                None
            }
        };

        let provider = SdkMeterProvider::builder()
            .with_reader(reader)
            .with_resource(get_resource())
            .with_view(http_client_request_duration_view)
            .build();

        global::set_meter_provider(provider);
    }

    {
        let exporter = SpanExporter::builder()
            .with_tonic()
            .build()
            .unwrap();

        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .with_resource(get_resource())
            .build();

        global::set_tracer_provider(provider);
    }

    let connector = hyper_util::client::legacy::connect::HttpConnector::new();
    let hyper_client = Client::builder(TokioExecutor::new()).build::<_, Empty<Bytes>>(connector);

    let otel_layer = LayerBuilder::builder().build().unwrap();

    let mut instrumented_client = ServiceBuilder::new()
        .layer(otel_layer)
        .service(hyper_client);

    let target_url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://localhost:5000/".to_string());

    println!("Sending requests to {target_url} until Ctrl-C...");

    loop {
        let req = Request::builder()
            .method("GET")
            .uri(&target_url)
            .body(Empty::<Bytes>::new())
            .unwrap();

        select! {
            res = instrumented_client.call(req) => {
                match res {
                    Ok(res) => {
                        let status = res.status();
                        let _body = res.into_body().collect().await.unwrap();
                        println!("response: {status}");
                    }
                    Err(err) => {
                        eprintln!("request error: {err}");
                    }
                }
            }
            _ = shutdown_signal() => {
                println!("shutting down");
                break;
            }
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
