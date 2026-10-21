use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use opentelemetry::{metrics::MeterProvider as _, KeyValue};
use opentelemetry_config::{providers::TelemetryProviders, ConfigurationProviderRegistry};
use opentelemetry_sdk::{
    error::OTelSdkResult,
    metrics::{
        data::{AggregatedMetrics, MetricData, ResourceMetrics},
        exporter::PushMetricExporter,
        MeterProviderBuilder, PeriodicReader, Temporality,
    },
};

#[derive(Default, Clone)]
struct CapturedData {
    resource_attributes: Vec<(String, String)>,
    metrics: Vec<(String, u64)>,
}

#[derive(Clone)]
struct TestMetricExporter {
    captured: Arc<Mutex<CapturedData>>,
}

impl PushMetricExporter for TestMetricExporter {
    async fn export(&self, resource_metrics: &ResourceMetrics) -> OTelSdkResult {
        let mut captured = self.captured.lock().unwrap();

        for (k, v) in resource_metrics.resource().iter() {
            captured
                .resource_attributes
                .push((k.as_str().to_string(), v.to_string()));
        }

        for scope in resource_metrics.scope_metrics() {
            for metric in scope.metrics() {
                let name = metric.name().to_string();
                if let AggregatedMetrics::U64(MetricData::Sum(sum)) = metric.data() {
                    for dp in sum.data_points() {
                        captured.metrics.push((name.clone(), dp.value()));
                    }
                }
            }
        }

        Ok(())
    }

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        self.shutdown()
    }

    fn temporality(&self) -> Temporality {
        Temporality::Cumulative
    }
}

#[test]
fn test_end_to_end_metric_recording_export_and_resource_attributes() {
    let captured = Arc::new(Mutex::new(CapturedData::default()));
    let captured_clone = Arc::clone(&captured);

    let mut registry = ConfigurationProviderRegistry::default();
    registry.register_metric_exporter_factory(
        "test_recorder",
        move |builder: MeterProviderBuilder, config: &str| {
            let config: serde_yaml::Value = serde_yaml::from_str(config).unwrap();
            assert_eq!(config["interval"].as_u64(), Some(10_000));
            assert_eq!(
                config["exporter"]["test_recorder"],
                serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
            );
            let exporter = TestMetricExporter {
                captured: Arc::clone(&captured_clone),
            };
            let reader = PeriodicReader::builder(exporter)
                .with_interval(Duration::from_millis(config["interval"].as_u64().unwrap()))
                .build();
            Ok(builder.with_reader(reader))
        },
    );

    let yaml_config = r#"
file_format: "1.2"
resource:
  attributes:
    - name: service.name
      value: "my-integration-service"
    - name: service.version
      value: "2.1.0"
    - name: deployment.environment
      value: "staging"
    - name: instance.count
      value: 8
      type: int
    - name: is_active
      value: true
      type: bool
    - name: ignored.attribute
      value: null
meter_provider:
  readers:
    - periodic:
        interval: 10000
        exporter:
          test_recorder: {}
"#;

    let providers = TelemetryProviders::configure_from_yaml_str(&registry, yaml_config).unwrap();
    let meter_provider = providers
        .meter_provider()
        .expect("MeterProvider should be configured");

    let meter = meter_provider.meter("test_meter");
    let counter = meter.u64_counter("requests_total").build();
    counter.add(99, &[KeyValue::new("endpoint", "/api/v1/resource")]);

    // Force flush ensures immediate synchronous export to our test exporter
    meter_provider.force_flush().unwrap();

    {
        let captured_data = captured.lock().unwrap();
        assert_eq!(captured_data.metrics.len(), 1);
        assert_eq!(captured_data.metrics[0], ("requests_total".to_string(), 99));

        let res_map: HashMap<_, _> = captured_data.resource_attributes.iter().cloned().collect();
        assert_eq!(
            res_map.get("service.name").map(|s| s.as_str()),
            Some("my-integration-service")
        );
        assert_eq!(
            res_map.get("service.version").map(|s| s.as_str()),
            Some("2.1.0")
        );
        assert_eq!(
            res_map.get("deployment.environment").map(|s| s.as_str()),
            Some("staging")
        );
        assert_eq!(res_map.get("instance.count").map(|s| s.as_str()), Some("8"));
        assert_eq!(res_map.get("is_active").map(|s| s.as_str()), Some("true"));
        assert!(!res_map.contains_key("ignored.attribute"));
    }

    // Clean shutdown
    meter_provider.shutdown().unwrap();
}

#[test]
fn test_end_to_end_console_exporter_initialization_and_shutdown() {
    if std::env::var("RUN_CONSOLE_TEST_CHILD").as_deref() == Ok("1") {
        let registry = ConfigurationProviderRegistry::default();
        let yaml_config = r#"
file_format: "1.2"
resource:
  attributes:
    - name: service.name
      value: "console-test-service"
meter_provider:
  readers:
    - periodic:
        interval: 60000
        exporter:
          console: {}
"#;

        let providers =
            TelemetryProviders::configure_from_yaml_str(&registry, yaml_config).unwrap();
        let meter_provider = providers
            .meter_provider()
            .expect("MeterProvider should be configured");

        let meter = meter_provider.meter("console_meter");
        let counter = meter.u64_counter("console_events").build();
        counter.add(42, &[KeyValue::new("status", "ok")]);

        // Flushing to stdout console exporter
        meter_provider.force_flush().unwrap();
        meter_provider.shutdown().unwrap();
        return;
    }

    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--nocapture")
        .arg("test_end_to_end_console_exporter_initialization_and_shutdown")
        .env("RUN_CONSOLE_TEST_CHILD", "1")
        .output()
        .expect("child process should execute successfully");

    assert!(
        output.status.success(),
        "Child test process failed:\nSTDOUT: {}\nSTDERR: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("console_events"),
        "Output did not contain metric name 'console_events'. Stdout:\n{}",
        stdout
    );
    assert!(
        stdout.contains("Value        : 42"),
        "Output did not contain metric value 42. Stdout:\n{}",
        stdout
    );
    assert!(
        stdout.contains("console-test-service"),
        "Output did not contain resource attribute 'console-test-service'. Stdout:\n{}",
        stdout
    );
}

#[test]
fn test_resource_does_not_implicitly_read_env_vars() {
    // If running in child process, execute the test assertion with environment variables active
    if std::env::var("RUN_RESOURCE_ENV_TEST_CHILD").as_deref() == Ok("1") {
        let captured = Arc::new(Mutex::new(CapturedData::default()));
        let captured_clone = Arc::clone(&captured);

        let mut registry = ConfigurationProviderRegistry::default();
        registry.register_metric_exporter_factory(
            "test_recorder",
            move |builder: MeterProviderBuilder, _config: &str| {
                let exporter = TestMetricExporter {
                    captured: Arc::clone(&captured_clone),
                };
                let reader = PeriodicReader::builder(exporter)
                    .with_interval(Duration::from_millis(60_000))
                    .build();
                Ok(builder.with_reader(reader))
            },
        );

        let yaml_config = r#"
file_format: "1.2"
resource:
  attributes:
    - name: service.name
      value: "yaml-configured-service"
    - name: custom.attr
      value: "yaml-value"
meter_provider:
  readers:
    - periodic:
        interval: 10000
        exporter:
          test_recorder: {}
"#;

        let providers =
            TelemetryProviders::configure_from_yaml_str(&registry, yaml_config).unwrap();
        let meter_provider = providers.meter_provider().unwrap();

        let meter = meter_provider.meter("test");
        let counter = meter.u64_counter("test_counter").build();
        counter.add(1, &[]);

        meter_provider.force_flush().unwrap();

        {
            let captured_data = captured.lock().unwrap();
            let res_map: HashMap<_, _> =
                captured_data.resource_attributes.iter().cloned().collect();

            // Must match YAML configured service name, not env
            assert_eq!(
                res_map.get("service.name").map(|s| s.as_str()),
                Some("yaml-configured-service")
            );
            assert_eq!(
                res_map.get("custom.attr").map(|s| s.as_str()),
                Some("yaml-value")
            );

            // Environment variables must NOT be present
            assert!(!res_map.contains_key("env.injected"));

            // Default SDK telemetry attributes must still be preserved
            assert_eq!(
                res_map.get("telemetry.sdk.name").map(|s| s.as_str()),
                Some("opentelemetry")
            );
            assert_eq!(
                res_map.get("telemetry.sdk.language").map(|s| s.as_str()),
                Some("rust")
            );
            assert!(res_map.contains_key("telemetry.sdk.version"));
        }

        meter_provider.shutdown().unwrap();
        return;
    }

    // Parent process: spawn child process with environment variables set on the Command only.
    // This avoids mutating process-wide variables in concurrently running tests, and does not
    // overwrite or discard any environment variables that existed before the test.
    let current_exe = std::env::current_exe().expect("current exe should be found");
    let output = std::process::Command::new(current_exe)
        .arg("--nocapture")
        .arg("test_resource_does_not_implicitly_read_env_vars")
        .env("RUN_RESOURCE_ENV_TEST_CHILD", "1")
        .env(
            "OTEL_RESOURCE_ATTRIBUTES",
            "env.injected=true,service.name=env-service",
        )
        .env("OTEL_SERVICE_NAME", "otel-env-service")
        .output()
        .expect("child process should execute successfully");

    assert!(
        output.status.success(),
        "Child test process failed:\nSTDOUT: {}\nSTDERR: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
