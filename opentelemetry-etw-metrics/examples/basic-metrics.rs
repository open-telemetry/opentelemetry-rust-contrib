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

const SERVICE_NAME: &str = "hello-geneva-user-events";
const METRICS_ACCOUNT: &str = "your-geneva-metrics-account";
const METRICS_NAMESPACE: &str = "your-geneva-metrics-namespace";

fn setup_meter_provider() -> SdkMeterProvider {
    let exporter = MetricsExporter::new();
    let reader = PeriodicReader::builder(exporter, runtime::Tokio)
        .with_interval(std::time::Duration::from_millis(1000))
        .build();
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

    // The name of the meter is ignored in Geneva today.
    let meter = meter_provider.meter("user-event-test");
    let c = meter
        .f64_counter("MyFruitCounter") // This will be the metric name in Geneva
        .with_description("test_description") // The description is ignored in Geneva today
        .with_unit(Unit::new("test_unit")) // The unit is ignored in Geneva today
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
