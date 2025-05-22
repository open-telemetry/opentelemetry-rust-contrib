//! run with `$ cargo run --example basic-metrics --all-features
use opentelemetry::{metrics::MeterProvider as _, KeyValue};
use opentelemetry_sdk::{metrics::SdkMeterProvider, Resource};
use opentelemetry_user_events_metrics::MetricsExporter;
use std::thread;
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

fn init_metrics(exporter: MetricsExporter) -> SdkMeterProvider {
    SdkMeterProvider::builder()
        .with_resource(
            Resource::builder_empty()
                .with_attributes(vec![KeyValue::new("service.name", "metric-demo")])
                .build(),
        )
        .with_periodic_exporter(exporter)
        .build()
}

#[tokio::main]
#[allow(unused_must_use)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a new tracing::Fmt layer to print the logs to stdout. It has a
    // default filter of `info` level and above, and `debug` and above for logs
    // from OpenTelemetry crates. The filter levels can be customized as needed.
    let filter_fmt = EnvFilter::new("info").add_directive("opentelemetry=debug".parse().unwrap());
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_thread_names(true)
        .with_filter(filter_fmt);

    tracing_subscriber::registry().with(fmt_layer).init();

    let exporter = opentelemetry_user_events_metrics::MetricsExporter::new();
    let meter_provider = init_metrics(exporter);

    let meter = meter_provider.meter("user-event-test");

    // Create a Counter Instrument.
    let counter = meter
        .f64_counter("counter_f64_test")
        .with_description("test_decription")
        .with_unit("test_unit")
        .build();

    let counter2 = meter
        .u64_counter("counter_u64_test")
        .with_description("test_decription")
        .with_unit("test_unit")
        .build();

    // Create an UpDownCounter Instrument.
    let updown_counter = meter
        .i64_up_down_counter("up_down_counter_i64_test")
        .build();
    let updown_counter2 = meter
        .f64_up_down_counter("up_down_counter_f64_test")
        .build();

    // Create a Histogram Instrument.
    let histogram = meter
        .f64_histogram("histogram_f64_test")
        .with_description("test_description")
        .build();
    let histogram2 = meter
        .u64_histogram("histogram_u64_test")
        .with_description("test_description")
        .build();

    // Create a ObservableGauge instrument and register a callback that reports the measurement.
    let _gauge = meter
        .f64_observable_gauge("observable_gauge_f64_test")
        .with_unit("test_unit")
        .with_description("test_description")
        .with_callback(|observer| {
            observer.observe(
                1.0,
                &[
                    KeyValue::new("mykey1", "myvalue1"),
                    KeyValue::new("mykey2", "myvalue2"),
                ],
            )
        })
        .build();

    let _gauge2 = meter
        .u64_observable_gauge("observable_gauge_u64_test")
        .with_unit("test_unit")
        .with_description("test_description")
        .with_callback(|observer| {
            observer.observe(
                1,
                &[
                    KeyValue::new("mykey1", "myvalue1"),
                    KeyValue::new("mykey2", "myvalue2"),
                ],
            )
        })
        .build();

    // Create a ObservableCounter instrument and register a callback that reports the measurement.
    let _observable_counter = meter
        .u64_observable_counter("observable_counter_u64_test")
        .with_description("test_description")
        .with_unit("test_unit")
        .with_callback(|observer| {
            observer.observe(
                100,
                &[
                    KeyValue::new("mykey1", "myvalue1"),
                    KeyValue::new("mykey2", "myvalue2"),
                ],
            )
        })
        .build();

    let _observable_counter2 = meter
        .f64_observable_counter("observable_counter_f64_test")
        .with_description("test_description")
        .with_unit("test_unit")
        .with_callback(|observer| {
            observer.observe(
                100.0,
                &[
                    KeyValue::new("mykey1", "myvalue1"),
                    KeyValue::new("mykey2", "myvalue2"),
                ],
            )
        })
        .build();

    // Create a Observable UpDownCounter instrument and register a callback that reports the measurement.
    let _observable_up_down_counter = meter
        .i64_observable_up_down_counter("observable_up_down_counter_i64_test")
        .with_description("test_description")
        .with_unit("test_unit")
        .with_callback(|observer| {
            observer.observe(
                100,
                &[
                    KeyValue::new("mykey1", "myvalue1"),
                    KeyValue::new("mykey2", "myvalue2"),
                ],
            )
        })
        .build();
    let _observable_up_down_counter2 = meter
        .f64_observable_up_down_counter("observable_up_down_counter_f64_test")
        .with_description("test_description")
        .with_unit("test_unit")
        .with_callback(|observer| {
            observer.observe(
                100.0,
                &[
                    KeyValue::new("mykey1", "myvalue1"),
                    KeyValue::new("mykey2", "myvalue2"),
                ],
            )
        })
        .build();

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
