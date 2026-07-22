// Purpose-built example for the weaver-live-check-tower CI workflow.
// Exercises the tower HTTP instrumentation across the full
// HTTP-method x status-code x route-shape matrix so that weaver can
// validate as many attribute combinations as possible against the
// pinned semantic-conventions registry.

use axum::{
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{any, delete, get, options, patch, post, put},
    Router,
};
use opentelemetry::global;
use opentelemetry_instrumentation_tower::http::server::LayerBuilder;
use opentelemetry_otlp::{MetricExporter, SpanExporter};
use opentelemetry_sdk::{
    metrics::{PeriodicReader, SdkMeterProvider},
    trace::SdkTracerProvider,
    Resource,
};
use std::sync::OnceLock;
use tower_http::catch_panic::CatchPanicLayer;

const SERVICE_NAME: &str = "example-live-check-app";

fn get_resource() -> Resource {
    static RESOURCE: OnceLock<Resource> = OnceLock::new();
    RESOURCE
        .get_or_init(|| Resource::builder().with_service_name(SERVICE_NAME).build())
        .clone()
}

// 200 OK handlers ---------------------------------------------------------

async fn root() -> &'static str {
    "ok"
}

async fn get_user(Path(id): Path<u32>) -> String {
    format!("user-{id}")
}

async fn get_file(Path(path): Path<String>) -> String {
    format!("file:{path}")
}

// 201 / 200 / 204 status code handlers ------------------------------------

async fn create_item() -> impl IntoResponse {
    (StatusCode::CREATED, "created")
}

async fn update_item(Path(id): Path<u32>) -> String {
    format!("updated {id}")
}

async fn patch_item(Path(id): Path<u32>) -> String {
    format!("patched {id}")
}

async fn delete_item(Path(_id): Path<u32>) -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn options_items() -> StatusCode {
    StatusCode::OK
}

// 4xx / 5xx handlers (handled errors) -------------------------------------

async fn bad_request() -> impl IntoResponse {
    (StatusCode::BAD_REQUEST, "bad input")
}

async fn unauthorized() -> impl IntoResponse {
    (StatusCode::UNAUTHORIZED, "")
}

async fn server_error() -> impl IntoResponse {
    (StatusCode::INTERNAL_SERVER_ERROR, "intentional")
}

// Intentional panic; CatchPanicLayer converts it to a 500 response.
async fn panic_path() -> StatusCode {
    panic!("intentional panic for live-check coverage")
}

// HEAD on a templated route via `any` to exercise an additional method.
async fn item_head(Path(_id): Path<u32>) -> StatusCode {
    StatusCode::OK
}

async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = async {
        signal::ctrl_c().await.expect("ctrl-c handler");
    };
    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(unix)]
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    #[cfg(not(unix))]
    ctrl_c.await;
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
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

    // The matrix exercised below:
    //   methods:      GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS
    //   status codes: 200, 201, 204, 400, 401, 404, 500
    //   route shapes: literal, templated param, wildcard catch-all
    //   error shape:  handled 4xx/5xx + unhandled panic
    let otel_layer = LayerBuilder::builder().build().unwrap();

    let app = Router::new()
        .route("/", get(root))
        .route("/users/{id}", get(get_user))
        .route("/items", post(create_item))
        .route("/items", options(options_items))
        .route("/items/{id}", put(update_item))
        .route("/items/{id}", patch(patch_item))
        .route("/items/{id}", delete(delete_item))
        // HEAD on /items/{id} - mapped via `any` so the same path supports
        // multiple method shapes that aren't already declared above.
        .route("/items/{id}/head", any(item_head))
        .route("/files/{*path}", get(get_file))
        .route("/bad-request", get(bad_request))
        .route("/unauthorized", get(unauthorized))
        .route("/error", get(server_error))
        .route("/throw", get(panic_path))
        // CatchPanicLayer must sit between the OTel layer and the handlers so
        // that a panicked handler is converted to a real 500 response, which
        // the outer OTel layer then records as a normal span/metric.
        .layer(CatchPanicLayer::new())
        .layer(otel_layer);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:5000")
        .await
        .expect("bind 0.0.0.0:5000");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");

    let _ = meter_provider.shutdown();
    let _ = tracer_provider.shutdown();
}
