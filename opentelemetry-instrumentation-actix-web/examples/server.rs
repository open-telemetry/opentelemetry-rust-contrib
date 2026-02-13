use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use opentelemetry::global;
use opentelemetry_instrumentation_actix_web::{RequestMetrics, RequestTracing};
use opentelemetry_sdk::{
    metrics::SdkMeterProvider, propagation::TraceContextPropagator, trace::SdkTracerProvider,
    Resource,
};
use opentelemetry_stdout::{MetricExporter, SpanExporter};
use std::sync::OnceLock;

const SERVICE_NAME: &str = "actix_server";

fn get_resource() -> Resource {
    static RESOURCE: OnceLock<Resource> = OnceLock::new();
    RESOURCE
        .get_or_init(|| Resource::builder().with_service_name(SERVICE_NAME).build())
        .clone()
}

async fn manual_hello() -> impl Responder {
    HttpResponse::Ok().body("Hey there!")
}

async fn get_user(user_id: web::Path<String>) -> impl Responder {
    HttpResponse::Ok().body(format!("User ID: {}", user_id))
}

#[actix_web::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Start a new OTLP trace pipeline
    global::set_text_map_propagator(TraceContextPropagator::new());

    let tracer_provider = SdkTracerProvider::builder()
        .with_simple_exporter(SpanExporter::default())
        .with_resource(get_resource())
        .build();

    global::set_tracer_provider(tracer_provider.clone());

    // Setup a OTLP metrics exporter if --features metrics is used
    #[cfg(feature = "metrics")]
    let meter_provider = {
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(MetricExporter::default())
            .with_resource(get_resource())
            .build();
        global::set_meter_provider(provider.clone());

        provider
    };

    HttpServer::new(move || {
        App::new()
            .wrap(RequestTracing::new())
            .wrap(RequestMetrics::default())
            .route("/hey", web::get().to(manual_hello))
            .route("/users/{user_id}", web::get().to(get_user))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await?;

    // Ensure all spans have been reported
    tracer_provider.shutdown()?;

    #[cfg(feature = "metrics")]
    meter_provider.shutdown()?;

    Ok(())
}
