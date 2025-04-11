use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use opentelemetry::{global, KeyValue};
use opentelemetry_instrumentation_actix_web::{RequestMetrics, RequestTracing};
use opentelemetry_sdk::{
    metrics::{Aggregation, Instrument, SdkMeterProvider, Stream},
    propagation::TraceContextPropagator,
    trace::SdkTracerProvider,
    Resource,
};
use opentelemetry_stdout::{MetricExporter, SpanExporter};

async fn manual_hello() -> impl Responder {
    HttpResponse::Ok().body("Hey there!")
}

#[actix_web::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Start a new OTLP trace pipeline
    global::set_text_map_propagator(TraceContextPropagator::new());

    let service_name_resource = Resource::builder_empty()
        .with_attribute(KeyValue::new("service.name", "actix_server"))
        .build();

    let tracer = SdkTracerProvider::builder()
        .with_simple_exporter(SpanExporter::default())
        .with_resource(service_name_resource)
        .build();

    global::set_tracer_provider(tracer.clone());

    // Setup a OTLP metrics exporter if --features metrics is used
    #[cfg(feature = "metrics")]
    let meter_provider = {
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(MetricExporter::default())
            .with_resource(
                Resource::builder_empty()
                    .with_attribute(KeyValue::new("service.name", "my_app"))
                    .build(),
            )
            .with_view(
                opentelemetry_sdk::metrics::new_view(
                    Instrument::new().name("http.server.duration"),
                    Stream::new().aggregation(Aggregation::ExplicitBucketHistogram {
                        boundaries: vec![
                            0.0, 0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5,
                            5.0, 7.5, 10.0,
                        ],
                        record_min_max: true,
                    }),
                )
                .unwrap(),
            )
            .build();
        global::set_meter_provider(provider.clone());

        provider
    };

    HttpServer::new(move || {
        App::new()
            .wrap(RequestTracing::new())
            .wrap(RequestMetrics::default())
            .route("/hey", web::get().to(manual_hello))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await?;

    // Ensure all spans have been reported
    tracer.shutdown()?;

    #[cfg(feature = "metrics")]
    meter_provider.shutdown()?;

    Ok(())
}
