//! run with `$ cargo run --example basic --all-features
use opentelemetry::{
    metrics::{MeterProvider as _, Unit},
    KeyValue,
};
use opentelemetry_etw_metrics::MetricsExporter;
use opentelemetry_sdk::{
    metrics::{PeriodicReader, SdkMeterProvider},
    runtime, Resource,
};
use std::{thread, time::Duration};

const SERVICE_NAME: &str = "service-name";
const METRICS_ACCOUNT: &str = "metrics-account";
const METRICS_NAMESPACE: &str = "metrics-namespace";

fn setup_meter_provider() -> SdkMeterProvider {
    let exporter = MetricsExporter::new();
    let reader = PeriodicReader::builder(exporter, runtime::Tokio).build();
    SdkMeterProvider::builder()
        .with_resource(Resource::new(vec![
            KeyValue::new("service.name", SERVICE_NAME),
            KeyValue::new("_metrics_account", METRICS_ACCOUNT),
            KeyValue::new("_metrics_namespace", METRICS_NAMESPACE),
        ]))
        .with_reader(reader)
        .build()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let meter_provider = setup_meter_provider();

    let meter = meter_provider.meter("user-event-test");
    let c = meter
        .f64_counter("MyFruitCounter")
        .with_description("test_description")
        .with_unit(Unit::new("test_unit"))
        .init();

    loop {
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

        // Sleep for 1 second
        thread::sleep(Duration::from_secs(1));
        println!("Running...");
    }

    meter_provider.shutdown()?;
    Ok(())
}
