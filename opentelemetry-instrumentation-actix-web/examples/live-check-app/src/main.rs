// Purpose-built example for the weaver-live-check-actix CI workflow.
// Exercises the actix-web HTTP instrumentation across the full
// HTTP-method x status-code x route-shape matrix so that weaver can
// validate as many attribute combinations as possible against the
// pinned semantic-conventions registry.

use actix_web::{http::Method, web, App, HttpResponse, HttpServer, Responder};
use opentelemetry::global;
use opentelemetry_instrumentation_actix_web::{RequestMetrics, RequestTracing};
use opentelemetry_otlp::{MetricExporter, SpanExporter};
use opentelemetry_sdk::{
    metrics::{PeriodicReader, SdkMeterProvider},
    trace::SdkTracerProvider,
    Resource,
};
use std::sync::OnceLock;

const SERVICE_NAME: &str = "example-live-check-actix";

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

async fn get_user(path: web::Path<u32>) -> String {
    format!("user-{}", path.into_inner())
}

async fn get_file(path: web::Path<String>) -> String {
    format!("file:{}", path.into_inner())
}

// 201 / 200 / 204 status code handlers ------------------------------------

async fn create_item() -> impl Responder {
    HttpResponse::Created().body("created")
}

async fn update_item(path: web::Path<u32>) -> String {
    format!("updated {}", path.into_inner())
}

async fn patch_item(path: web::Path<u32>) -> String {
    format!("patched {}", path.into_inner())
}

async fn delete_item(_path: web::Path<u32>) -> HttpResponse {
    HttpResponse::NoContent().finish()
}

async fn options_items() -> HttpResponse {
    HttpResponse::Ok().finish()
}

// 4xx / 5xx handlers (handled errors) -------------------------------------

async fn bad_request() -> impl Responder {
    HttpResponse::BadRequest().body("bad input")
}

async fn unauthorized() -> impl Responder {
    HttpResponse::Unauthorized().finish()
}

async fn server_error() -> impl Responder {
    HttpResponse::InternalServerError().body("intentional")
}

// HEAD on a templated route to exercise an additional method.
async fn item_head(_path: web::Path<u32>) -> HttpResponse {
    HttpResponse::Ok().finish()
}

// NOTE: tower's live-check-app also exercises a panicking handler (converted to
// a 500 response via `tower-http`'s `CatchPanicLayer`, then recorded by the
// OTel layer). The actix-web instrumentation does not currently emit a span or
// record metrics when a handler panics — the future panics before the
// middleware response wrapper runs. The /error path already exercises 5xx
// attribute coverage, so a panicking handler is intentionally omitted here.

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

#[actix_web::main]
async fn main() -> std::io::Result<()> {
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
    let server = HttpServer::new(|| {
        App::new()
            .wrap(RequestTracing::new())
            .wrap(RequestMetrics::default())
            .route("/", web::get().to(root))
            .route("/users/{id}", web::get().to(get_user))
            .route("/items", web::post().to(create_item))
            .route("/items", web::method(Method::OPTIONS).to(options_items))
            .route("/items/{id}", web::put().to(update_item))
            .route("/items/{id}", web::patch().to(patch_item))
            .route("/items/{id}", web::delete().to(delete_item))
            .route("/items/{id}/head", web::head().to(item_head))
            .route("/files/{path:.*}", web::get().to(get_file))
            .route("/bad-request", web::get().to(bad_request))
            .route("/unauthorized", web::get().to(unauthorized))
            .route("/error", web::get().to(server_error))
    })
    // Single worker keeps a panic from racing against shutdown signal delivery.
    .workers(1)
    .bind(("0.0.0.0", 5000))?
    .shutdown_timeout(2)
    .run();

    let server_handle = server.handle();
    let server_task = tokio::spawn(server);

    shutdown_signal().await;
    server_handle.stop(true).await;
    let _ = server_task.await;

    let _ = meter_provider.shutdown();
    let _ = tracer_provider.shutdown();

    Ok(())
}
