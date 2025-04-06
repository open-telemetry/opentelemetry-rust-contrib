use actix_web::{web, App, HttpRequest, HttpServer};
use opentelemetry::{global, KeyValue};
use opentelemetry_instrumentation_actix_web::{
    PrometheusMetricsHandler, RequestMetrics, RequestTracing,
};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    metrics::{Aggregation, Instrument, SdkMeterProvider, Stream},
    propagation::TraceContextPropagator,
    trace::SdkTracerProvider,
    Resource,
};

async fn index(_req: HttpRequest, _path: actix_web::web::Path<String>) -> &'static str {
    "Hello world!"
}

#[actix_web::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Start a new OTLP trace pipeline
    global::set_text_map_propagator(TraceContextPropagator::new());

    let service_name_resource = Resource::builder_empty()
        .with_attribute(KeyValue::new("service.name", "actix_server"))
        .build();

    let tracer = SdkTracerProvider::builder()
        .with_batch_exporter(
            opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint("http://127.0.0.1:6565")
                .build()?,
        )
        .with_resource(service_name_resource)
        .build();

    global::set_tracer_provider(tracer.clone());

    // Start a new prometheus metrics pipeline if --features metrics-prometheus is used
    #[cfg(feature = "metrics-prometheus")]
    let (metrics_handler, meter_provider) = {
        let registry = prometheus::Registry::new();
        let exporter = opentelemetry_prometheus::exporter()
            .with_registry(registry.clone())
            .build()?;

        let provider = SdkMeterProvider::builder()
            .with_reader(exporter)
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

        (PrometheusMetricsHandler::new(registry), provider)
    };

    HttpServer::new(move || {
        let app = App::new()
            .wrap(RequestTracing::new())
            .wrap(RequestMetrics::default())
            .service(web::resource("/users/{id}").to(index));

        #[cfg(feature = "metrics-prometheus")]
        let app = app.route("/metrics", web::get().to(metrics_handler.clone()));

        app
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await?;

    // Ensure all spans have been reported
    tracer.shutdown()?;

    #[cfg(feature = "metrics-prometheus")]
    meter_provider.shutdown()?;

    Ok(())
}
