//! run with `$ cargo run --example basic --all-features
use opentelemetry::{
    metrics::{MeterProvider as _, Unit},
    KeyValue,
};
use opentelemetry_sdk::{
    metrics::{MeterProvider as SdkMeterProvider, PeriodicReader},
    runtime, Resource,
};
use opentelemetry_user_events_metrics::MetricsExporter;
use std::thread;
use std::time::Duration;

fn init_metrics(exporter: MetricsExporter) -> SdkMeterProvider {
    let reader = PeriodicReader::builder(exporter, runtime::Tokio).build();
    SdkMeterProvider::builder()
        .with_resource(Resource::new(vec![KeyValue::new(
            "service.name",
            "metric-demo",
        )]))
        .with_reader(reader)
        .build()
}

#[tokio::main]
#[allow(unused_must_use)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let exporter = opentelemetry_user_events_metrics::MetricsExporter::new();
    let meter_provider = init_metrics(exporter);

    let meter = meter_provider.versioned_meter(
        "user-event-test",
        Some("test-version"),
        Some("test_url"),
        Some(vec![KeyValue::new("key", "value")]),
    );
    // Create a Counter Instrument.
    let counter = meter
        .f64_counter("counter_test")
        .with_description("test_decription")
        .with_unit(Unit::new("test_unit"))
        .init();
    // Create a UpCounter Instrument.
    let updown_counter = meter.i64_up_down_counter("up_down_counter_test").init();

    // Create a Histogram Instrument.
    let histogram = meter
        .f64_histogram("histogram_test")
        .with_description("test_description")
        .init();

    // Create a ObservableGauge instrument and register a callback that reports the measurement.
    let gauge = meter
        .f64_observable_gauge("gauge_test")
        .with_unit(Unit::new("test_unit"))
        .with_description("test_descriptionn")
        .init();

    meter.register_callback(&[gauge.as_any()], move |observer| {
        observer.observe_f64(
            &gauge,
            1.0,
            [
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ]
            .as_ref(),
        )
    })?;

    // Create a ObservableCounter instrument and register a callback that reports the measurement.
    let observable_counter = meter
        .u64_observable_counter("obs_counter_test")
        .with_description("test_description")
        .with_unit(Unit::new("tesT_unit"))
        .init();

    meter.register_callback(&[observable_counter.as_any()], move |observer| {
        observer.observe_u64(
            &observable_counter,
            100,
            [
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ]
            .as_ref(),
        )
    })?;

    // Create a Observable UpDownCounter instrument and register a callback that reports the measurement.
    let observable_up_down_counter = meter
        .i64_observable_up_down_counter("obs_up_down_counter_test")
        .with_description("test_description")
        .with_unit(Unit::new("test_unit"))
        .init();

    meter.register_callback(&[observable_up_down_counter.as_any()], move |observer| {
        observer.observe_i64(
            &observable_up_down_counter,
            100,
            [
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ]
            .as_ref(),
        )
    })?;

    loop {
        // Record measurements using the Counter instrument.
        counter.add(
            1.0,
            [
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ]
            .as_ref(),
        );

        // Record measurements using the UpCounter instrument.
        updown_counter.add(
            -10,
            [
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ]
            .as_ref(),
        );

        // Record measurements using the histogram instrument.
        histogram.record(
            10.5,
            [
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ]
            .as_ref(),
        );
        // Sleep for 1 second
        thread::sleep(Duration::from_secs(1));
    }

}
