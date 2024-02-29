//! run with `$ cargo run --example advanced --all-features
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

fn init_metrics(exporter: MetricsExporter) -> SdkMeterProvider {
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
#[allow(unused_must_use)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let exporter = opentelemetry_etw_metrics::MetricsExporter::new();
    let meter_provider = init_metrics(exporter);

    let meter = meter_provider.meter("etw-test");

    // Create a Counter Instrument.
    let counter = meter
        .f64_counter("counter_f64_test")
        .with_description("test_decription")
        .with_unit(Unit::new("test_unit"))
        .init();

    let counter2 = meter
        .u64_counter("counter_u64_test")
        .with_description("test_decription")
        .with_unit(Unit::new("test_unit"))
        .init();

    // Create an UpDownCounter Instrument.
    let updown_counter = meter.i64_up_down_counter("up_down_counter_i64_test").init();
    let updown_counter2 = meter.f64_up_down_counter("up_down_counter_f64_test").init();

    // Create a Histogram Instrument.
    let histogram = meter
        .f64_histogram("histogram_f64_test")
        .with_description("test_description")
        .init();
    let histogram2 = meter
        .u64_histogram("histogram_u64_test")
        .with_description("test_description")
        .init();

    // Create a ObservableGauge instrument and register a callback that reports the measurement.
    let gauge = meter
        .f64_observable_gauge("observable_gauge_f64_test")
        .with_unit(Unit::new("test_unit"))
        .with_description("test_description")
        .init();

    let gauge2 = meter
        .u64_observable_gauge("observable_gauge_u64_test")
        .with_unit(Unit::new("test_unit"))
        .with_description("test_description")
        .init();

    meter.register_callback(&[gauge.as_any()], move |observer| {
        observer.observe_f64(
            &gauge,
            1.0,
            &[
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ],
        )
    })?;

    meter.register_callback(&[gauge2.as_any()], move |observer| {
        observer.observe_u64(
            &gauge2,
            1,
            &[
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ],
        )
    })?;

    // Create a ObservableCounter instrument and register a callback that reports the measurement.
    let observable_counter = meter
        .u64_observable_counter("observable_counter_u64_test")
        .with_description("test_description")
        .with_unit(Unit::new("test_unit"))
        .init();

    let observable_counter2 = meter
        .f64_observable_counter("observable_counter_f64_test")
        .with_description("test_description")
        .with_unit(Unit::new("test_unit"))
        .init();

    meter.register_callback(&[observable_counter.as_any()], move |observer| {
        observer.observe_u64(
            &observable_counter,
            100,
            &[
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ],
        )
    })?;

    meter.register_callback(&[observable_counter2.as_any()], move |observer| {
        observer.observe_f64(
            &observable_counter2,
            100.0,
            &[
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ],
        )
    })?;

    // Create a Observable UpDownCounter instrument and register a callback that reports the measurement.
    let observable_up_down_counter = meter
        .i64_observable_up_down_counter("observable_up_down_counter_i64_test")
        .with_description("test_description")
        .with_unit(Unit::new("test_unit"))
        .init();
    let observable_up_down_counter2 = meter
        .f64_observable_up_down_counter("observable_up_down_counter_f64_test")
        .with_description("test_description")
        .with_unit(Unit::new("test_unit"))
        .init();

    meter.register_callback(&[observable_up_down_counter.as_any()], move |observer| {
        observer.observe_i64(
            &observable_up_down_counter,
            100,
            &[
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ],
        )
    })?;

    meter.register_callback(&[observable_up_down_counter2.as_any()], move |observer| {
        observer.observe_f64(
            &observable_up_down_counter2,
            100.0,
            &[
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ],
        )
    })?;

    loop {
        // Record measurements using the Counter instrument.
        counter.add(
            1.0,
            &[
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ],
        );

        counter2.add(
            1,
            &[
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ],
        );

        // Record measurements using the UpCounter instrument.
        updown_counter.add(
            10,
            &[
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ],
        );

        updown_counter2.add(
            10.0,
            &[
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ],
        );

        // Record measurements using the histogram instrument.
        histogram.record(
            10.5,
            &[
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ],
        );

        histogram2.record(
            10,
            &[
                KeyValue::new("mykey1", "myvalue1"),
                KeyValue::new("mykey2", "myvalue2"),
            ],
        );

        // Sleep for 1 second
        thread::sleep(Duration::from_secs(1));
    }
}
