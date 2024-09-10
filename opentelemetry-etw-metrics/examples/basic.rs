//! run with `$ cargo run --example basic --all-features
use opentelemetry::{global, metrics::MeterProvider as _, KeyValue};
use opentelemetry_etw_metrics::MetricsExporter;
use opentelemetry_sdk::{
    metrics::{PeriodicReader, SdkMeterProvider},
    runtime, Resource,
};

const SERVICE_NAME: &str = "service-name";

fn setup_meter_provider() -> SdkMeterProvider {
    let exporter = MetricsExporter::new();
    let reader = PeriodicReader::builder(exporter, runtime::Tokio).build();
    SdkMeterProvider::builder()
        .with_resource(Resource::new(vec![KeyValue::new(
            "service.name",
            SERVICE_NAME,
        )]))
        .with_reader(reader)
        .build()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let meter_provider = setup_meter_provider();
    global::set_meter_provider(meter_provider.clone());

    let meter = meter_provider.meter("user-event-test");
    let c = meter
        .f64_counter("MyFruitCounter")
        .with_description("test_description")
        .with_unit("test_unit")
        .init();

    c.add(
        1.0,
        [
            KeyValue::new("name", "apple"),
            KeyValue::new("color", "red"),
        ]
        .as_ref(),
    );
    c.add(
        2.0,
        [
            KeyValue::new("name", "lemon"),
            KeyValue::new("color", "yellow"),
        ]
        .as_ref(),
    );
    c.add(
        1.0,
        [
            KeyValue::new("name", "lemon"),
            KeyValue::new("color", "yellow"),
        ]
        .as_ref(),
    );
    c.add(
        2.0,
        [
            KeyValue::new("name", "apple"),
            KeyValue::new("color", "green"),
        ]
        .as_ref(),
    );
    c.add(
        5.0,
        [
            KeyValue::new("name", "apple"),
            KeyValue::new("color", "red"),
        ]
        .as_ref(),
    );
    c.add(
        4.0,
        [
            KeyValue::new("name", "lemon"),
            KeyValue::new("color", "yellow"),
        ]
        .as_ref(),
    );

    meter_provider.shutdown()?;
    Ok(())
}
