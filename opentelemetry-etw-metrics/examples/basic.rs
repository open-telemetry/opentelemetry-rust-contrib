//! run with `$ cargo run --example basic
use opentelemetry::{global, metrics::MeterProvider as _, KeyValue};
use opentelemetry_etw_metrics::MetricsExporter;
use opentelemetry_sdk::{
    metrics::{PeriodicReader, SdkMeterProvider},
    Resource,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

const SERVICE_NAME: &str = "service-name";

fn setup_meter_provider() -> SdkMeterProvider {
    let exporter = MetricsExporter::new();
    let reader = PeriodicReader::builder(exporter).build();
    SdkMeterProvider::builder()
        .with_resource(
            Resource::builder()
                .with_attributes(vec![KeyValue::new("service.name", SERVICE_NAME)])
                .build(),
        )
        .with_reader(reader)
        .build()
}

#[tokio::main]
async fn main() {
    // Enable tracing::fmt layer for viewing internal logs
    let filter_fmt = EnvFilter::new("info").add_directive("opentelemetry=debug".parse().unwrap());
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_thread_names(true)
        .with_filter(filter_fmt);

    tracing_subscriber::registry().with(fmt_layer).init();
    let meter_provider = setup_meter_provider();
    global::set_meter_provider(meter_provider.clone());

    let meter = meter_provider.meter("user-event-test");
    let c = meter
        .f64_counter("MyFruitCounter")
        .with_description("test_description")
        .with_unit("test_unit")
        .build();

    c.add(
        1.0,
        &[
            KeyValue::new("name", "apple"),
            KeyValue::new("color", "red"),
        ],
    );
    c.add(
        2.0,
        &[
            KeyValue::new("name", "lemon"),
            KeyValue::new("color", "yellow"),
        ],
    );
    c.add(
        1.0,
        &[
            KeyValue::new("name", "lemon"),
            KeyValue::new("color", "yellow"),
        ],
    );
    c.add(
        2.0,
        &[
            KeyValue::new("name", "apple"),
            KeyValue::new("color", "green"),
        ],
    );
    c.add(
        5.0,
        &[
            KeyValue::new("name", "apple"),
            KeyValue::new("color", "red"),
        ],
    );
    c.add(
        4.0,
        &[
            KeyValue::new("name", "lemon"),
            KeyValue::new("color", "yellow"),
        ],
    );

    if let Err(e) = meter_provider.shutdown() {
        println!("Error shutting down meter provider: {e:?}");
    }
}
