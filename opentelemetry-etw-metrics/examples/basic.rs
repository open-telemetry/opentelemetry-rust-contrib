//! run with `$ cargo run --example basic
use opentelemetry::{metrics::MeterProvider as _, KeyValue};
use opentelemetry_etw_metrics::MetricsExporter;
use opentelemetry_sdk::{
    metrics::{PeriodicReader, SdkMeterProvider},
    runtime, Resource,
};
use std::{thread, time::Duration};

const SERVICE_NAME: &str = "service-name";
const METRICS_ACCOUNT: &str = "cijo-account";
const METRICS_NAMESPACE: &str = "cijo-namespace"; // The namespace will be automatically created by Geneva metrics backend, if not existing.

fn setup_meter_provider() -> SdkMeterProvider {
    let exporter = MetricsExporter::new();
    let reader = PeriodicReader::builder(exporter, runtime::Tokio).with_interval(Duration::from_secs(1)).build();
    SdkMeterProvider::builder()
        .with_resource(Resource::new(vec![
            KeyValue::new("service.name", SERVICE_NAME),
            KeyValue::new("_microsoft_metrics_account", METRICS_ACCOUNT),
            KeyValue::new("_microsoft_metrics_namespace", METRICS_NAMESPACE),
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
        .with_unit("test_unit")
        .init();

    let c2 = meter
    .f64_counter("MyFruitCounter")
    .with_description("test_description")
    .with_unit("test_unit")
    .init();

    loop {
        for v in 0..1000 {
            c.add(10.0, &[KeyValue::new("A", v.to_string())]);
        }
    
        for v in 0..1000 {
            c2.add(10.0, &[KeyValue::new("A", v.to_string())]);
        }
    }
    

    meter_provider.shutdown()?;
    Ok(())
}
