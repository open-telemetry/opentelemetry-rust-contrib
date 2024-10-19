use opentelemetry::{
    global::{self, shutdown_tracer_provider},
    trace::{Span, TraceContextExt, Tracer},
    Key, KeyValue, Value,
};
use opentelemetry_datadog::{new_pipeline, ApiVersion};
use std::thread;
use std::time::Duration;

fn bar() {
    let tracer = global::tracer("component-bar");
    let mut span = tracer.start("bar");
    span.set_attribute(KeyValue::new(
        Key::new("span.type"),
        Value::String("sql".into()),
    ));
    span.set_attribute(KeyValue::new(
        Key::new("sql.query"),
        Value::String("SELECT * FROM table".into()),
    ));
    thread::sleep(Duration::from_millis(6));
    span.end()
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let tracer = new_pipeline()
        .with_service_name("trace-demo")
        .with_api_version(ApiVersion::Version05)
        .install_simple()?;

    tracer.in_span("foo", |cx| {
        let span = cx.span();
        span.set_attribute(KeyValue::new(
            Key::new("span.type"),
            Value::String("web".into()),
        ));
        span.set_attribute(KeyValue::new(
            Key::new("http.url"),
            Value::String("http://localhost:8080/foo".into()),
        ));
        span.set_attribute(KeyValue::new(
            Key::new("http.method"),
            Value::String("GET".into()),
        ));
        span.set_attribute(KeyValue::new(Key::new("http.status_code"), Value::I64(200)));

        thread::sleep(Duration::from_millis(6));
        bar();
        thread::sleep(Duration::from_millis(6));
    });

    shutdown_tracer_provider();

    Ok(())
}
