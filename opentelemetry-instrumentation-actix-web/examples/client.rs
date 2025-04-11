use opentelemetry::{global, KeyValue};
use opentelemetry_instrumentation_actix_web::ClientExt;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use opentelemetry_stdout::SpanExporter;
use std::error::Error;
use std::io;

async fn execute_request(client: awc::Client) -> io::Result<String> {
    let mut response = client
        .get("http://127.0.0.1:8080/users/103240ba-3d8d-4695-a176-e19cbc627483?a=1")
        .trace_request()
        .send()
        .await
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;

    let bytes = response
        .body()
        .await
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;

    std::str::from_utf8(&bytes)
        .map(|s| s.to_owned())
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
}

#[actix_web::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    // Start a new OTLP trace pipeline
    global::set_text_map_propagator(TraceContextPropagator::new());

    let service_name_resource = Resource::builder_empty()
        .with_attribute(KeyValue::new("service.name", "actix_client"))
        .build();

    let tracer = SdkTracerProvider::builder()
        .with_batch_exporter(SpanExporter::default())
        .with_resource(service_name_resource)
        .build();

    let client = awc::Client::new();
    let response = execute_request(client).await?;

    println!("Response: {}", response);

    tracer.shutdown()?;

    Ok(())
}
