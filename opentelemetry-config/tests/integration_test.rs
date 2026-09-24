use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use opentelemetry::{metrics::MeterProvider as _, KeyValue};
use opentelemetry_config::{
    providers::TelemetryProviders, ConfigurationProviderRegistry, ProviderError,
};
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
        .expect("Failed to execute child process for console export test");

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
fn test_validation_missing_file_format() {
    let registry = ConfigurationProviderRegistry::default();
    let yaml = r#"
meter_provider:
  readers:
    - periodic:
        exporter:
          console: {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required field `file_format`"));
}

#[test]
fn test_validation_unsupported_file_format() {
    let registry = ConfigurationProviderRegistry::default();
    let yaml = r#"
file_format: "1.1"
meter_provider:
  readers:
    - periodic:
        exporter:
          console: {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err
        .to_string()
        .contains("Unsupported file format version '1.1'"));
}

#[test]
fn test_validation_old_metrics_format_rejected() {
    let registry = ConfigurationProviderRegistry::default();
    let yaml = r#"
file_format: "1.2"
metrics:
  readers:
    - periodic:
        exporter:
          console: {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err
        .to_string()
        .contains("Old 'metrics' configuration format is not supported"));
}

#[test]
fn test_validation_missing_readers() {
    let registry = ConfigurationProviderRegistry::default();
    let yaml = r#"
file_format: "1.2"
meter_provider: {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required field `meter_provider.readers`"));
}

#[test]
fn test_validation_empty_readers() {
    let registry = ConfigurationProviderRegistry::default();
    let yaml = r#"
file_format: "1.2"
meter_provider:
  readers: []
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err.to_string().contains("must contain at least one reader"));
}

#[test]
fn test_validation_reader_no_variant() {
    let registry = ConfigurationProviderRegistry::default();
    let yaml = r#"
file_format: "1.2"
meter_provider:
  readers:
    - {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err.to_string().contains("found 0"));
}

#[test]
fn test_validation_reader_multiple_variants() {
    let registry = ConfigurationProviderRegistry::default();
    let yaml = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          console: {}
      pull:
        exporter:
          prometheus: {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err.to_string().contains("found 2"));
}

#[test]
fn test_validation_periodic_no_exporter() {
    let registry = ConfigurationProviderRegistry::default();
    let yaml = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        interval: 1000
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err
        .to_string()
        .contains("Missing required field `exporter`"));
}

#[test]
fn test_validation_periodic_empty_exporter() {
    let registry = ConfigurationProviderRegistry::default();
    let yaml = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter: {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err.to_string().contains("found 0"));
}

#[test]
fn test_validation_periodic_multiple_exporters() {
    let registry = ConfigurationProviderRegistry::default();
    let yaml = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          console: {}
          custom: {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err.to_string().contains("found 2"));
}

#[test]
fn test_validation_negative_interval() {
    let registry = ConfigurationProviderRegistry::default();
    let yaml = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        interval: -100
        exporter:
          console: {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err.to_string().contains("must be non-negative"));
}

#[test]
fn test_validation_wrong_type_interval() {
    let registry = ConfigurationProviderRegistry::default();
    let yaml = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        interval: "fast"
        exporter:
          console: {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err
        .to_string()
        .contains("`periodic.interval` must be a non-negative integer"));
}

#[test]
fn test_validation_explicit_timeout_rejected() {
    let registry = ConfigurationProviderRegistry::default();
    let yaml = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        interval: 1000
        timeout: 500
        exporter:
          console: {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err
        .to_string()
        .contains("periodic reader timeout is not supported"));
}

#[test]
fn test_validation_pull_reader_rejected() {
    let registry = ConfigurationProviderRegistry::default();
    let yaml = r#"
file_format: "1.2"
meter_provider:
  readers:
    - pull:
        exporter:
          prometheus: {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err.to_string().contains("Pull readers are not supported"));
}

#[test]
fn test_validation_unsupported_standard_exporters() {
    let registry = ConfigurationProviderRegistry::default();
    for exporter in &["otlp_http", "otlp_grpc", "prometheus"] {
        let yaml = format!(
            r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          {}: {{}}
"#,
            exporter
        );
        let err = TelemetryProviders::configure_from_yaml_str(&registry, &yaml).unwrap_err();
        match err {
            ProviderError::UnsupportedExporter(msg) => {
                assert!(msg.contains(exporter));
            }
            other => panic!(
                "Expected UnsupportedExporter for {}, got {:?}",
                exporter, other
            ),
        }
    }
}

#[test]
fn test_validation_unregistered_custom_exporter() {
    let registry = ConfigurationProviderRegistry::default();
    let yaml = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          non_existent_exporter: {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    match err {
        ProviderError::NotRegisteredProvider(msg) => {
            assert!(msg.contains("non_existent_exporter"));
        }
        other => panic!("Expected NotRegisteredProvider, got {:?}", other),
    }
}

#[test]
fn test_validation_unsupported_meter_provider_fields() {
    let registry = ConfigurationProviderRegistry::default();
    let yaml_views = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          console: {}
  views: []
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_views).unwrap_err();
    assert!(err
        .to_string()
        .contains("`meter_provider.views` is not supported"));

    let yaml_exemplar = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          console: {}
  exemplar_filter: trace_based
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_exemplar).unwrap_err();
    assert!(err
        .to_string()
        .contains("`meter_provider.exemplar_filter` is not supported"));
}

#[test]
fn test_validation_resource_attributes_type_mismatch() {
    let registry = ConfigurationProviderRegistry::default();

    // String declared, number provided
    let yaml_string_err = r#"
file_format: "1.2"
resource:
  attributes:
    - name: service.name
      value: 123
      type: string
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_string_err).unwrap_err();
    assert!(err
        .to_string()
        .contains("declared as type 'string' but value is not a string"));

    // Bool declared, string provided
    let yaml_bool_err = r#"
file_format: "1.2"
resource:
  attributes:
    - name: is_active
      value: "true"
      type: bool
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_bool_err).unwrap_err();
    assert!(err
        .to_string()
        .contains("declared as type 'bool' but value is not a boolean"));

    // Int declared, string provided
    let yaml_int_err = r#"
file_format: "1.2"
resource:
  attributes:
    - name: count
      value: "10"
      type: int
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_int_err).unwrap_err();
    assert!(err
        .to_string()
        .contains("declared as type 'int' but value is not an integer"));

    // Array type unsupported
    let yaml_array_err = r#"
file_format: "1.2"
resource:
  attributes:
    - name: tags
      value: ["a", "b"]
      type: string_array
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_array_err).unwrap_err();
    assert!(err
        .to_string()
        .contains("uses array type 'string_array' which is not supported"));

    // Array value unsupported
    let yaml_raw_array_err = r#"
file_format: "1.2"
resource:
  attributes:
    - name: tags
      value: [1, 2]
"#;
    let err =
        TelemetryProviders::configure_from_yaml_str(&registry, yaml_raw_array_err).unwrap_err();
    assert!(err
        .to_string()
        .contains("declared as type 'string' but value is not a string"));

    // The schema permits repeated names; the SDK retains the last non-null value.
    let yaml_dup = r#"
file_format: "1.2"
resource:
  attributes:
    - name: service.name
      value: "one"
    - name: service.name
      value: "two"
"#;
    TelemetryProviders::configure_from_yaml_str(&registry, yaml_dup).unwrap();
}

#[test]
fn test_validation_unsupported_resource_fields() {
    let registry = ConfigurationProviderRegistry::default();
    let yaml_detection = r#"
file_format: "1.2"
resource:
  detection/development: {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_detection).unwrap_err();
    assert!(err
        .to_string()
        .contains("`resource.detection` is not supported"));

    let yaml_schema = r#"
file_format: "1.2"
resource:
  schema_url: "https://opentelemetry.io/schemas/1.2.0"
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_schema).unwrap_err();
    assert!(err
        .to_string()
        .contains("`resource.schema_url` is not supported"));

    let yaml_attr_list = r#"
file_format: "1.2"
resource:
  attributes_list: "key=val"
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_attr_list).unwrap_err();
    assert!(err
        .to_string()
        .contains("`resource.attributes_list` is not supported"));
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

#[test]
fn test_validation_explicit_null_on_objects_rejected() {
    let registry = ConfigurationProviderRegistry::default();

    // meter_provider: null
    let yaml_null_mp = r#"
file_format: "1.2"
meter_provider: null
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_null_mp).unwrap_err();
    assert!(err
        .to_string()
        .contains("Field 'meter_provider' must be an object, but got null"));

    // meter_provider: ~ (YAML tilde null)
    let yaml_tilde_mp = r#"
file_format: "1.2"
meter_provider: ~
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_tilde_mp).unwrap_err();
    assert!(err
        .to_string()
        .contains("Field 'meter_provider' must be an object, but got null"));

    // resource: null
    let yaml_null_res = r#"
file_format: "1.2"
resource: null
meter_provider:
  readers:
    - periodic:
        exporter:
          console: {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_null_res).unwrap_err();
    assert!(err
        .to_string()
        .contains("Field 'resource' must be an object, but got null"));

    // periodic: null
    let yaml_null_periodic = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic: null
"#;
    let err =
        TelemetryProviders::configure_from_yaml_str(&registry, yaml_null_periodic).unwrap_err();
    assert!(err
        .to_string()
        .contains("Field 'periodic' must be an object, but got null"));

    // exporter: null
    let yaml_null_exporter = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter: null
"#;
    let err =
        TelemetryProviders::configure_from_yaml_str(&registry, yaml_null_exporter).unwrap_err();
    assert!(err
        .to_string()
        .contains("Field 'exporter' must be an object, but got null"));

    // readers: null
    let yaml_null_readers = r#"
file_format: "1.2"
meter_provider:
  readers: null
"#;
    let err =
        TelemetryProviders::configure_from_yaml_str(&registry, yaml_null_readers).unwrap_err();
    assert!(err
        .to_string()
        .contains("Field 'meter_provider.readers' must be an array, but got null"));

    // attributes: null
    let yaml_null_attrs = r#"
file_format: "1.2"
resource:
  attributes: null
meter_provider:
  readers:
    - periodic:
        exporter:
          console: {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_null_attrs).unwrap_err();
    assert!(err
        .to_string()
        .contains("Field 'resource.attributes' must be an array, but got null"));
}

#[test]
fn test_validation_allowed_null_fields() {
    let registry = ConfigurationProviderRegistry::default();

    // 1. console: null is allowed by schema v1.2.0 (defaults to standard console exporter)
    let yaml_console_null = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          console: null
"#;
    let providers =
        TelemetryProviders::configure_from_yaml_str(&registry, yaml_console_null).unwrap();
    assert!(providers.meter_provider().is_some());

    // 2. periodic.interval: null is allowed by schema v1.2.0 (defaults to 60000 ms)
    let yaml_interval_null = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        interval: null
        exporter:
          console: {}
"#;
    let providers =
        TelemetryProviders::configure_from_yaml_str(&registry, yaml_interval_null).unwrap();
    assert!(providers.meter_provider().is_some());

    // 3. console.temporality_preference: null is allowed by schema v1.2.0 (defaults to cumulative)
    let yaml_temp_null = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          console:
            temporality_preference: null
"#;
    let providers = TelemetryProviders::configure_from_yaml_str(&registry, yaml_temp_null).unwrap();
    assert!(providers.meter_provider().is_some());

    // 4. console.default_histogram_aggregation: null is allowed by schema v1.2.0 (defaults to explicit_bucket_histogram)
    let yaml_hist_null = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          console:
            default_histogram_aggregation: null
"#;
    let providers = TelemetryProviders::configure_from_yaml_str(&registry, yaml_hist_null).unwrap();
    assert!(providers.meter_provider().is_some());

    // 5. console.default_histogram_aggregation: explicit_bucket_histogram is explicitly supported
    let yaml_hist_explicit = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          console:
            default_histogram_aggregation: explicit_bucket_histogram
"#;
    let providers =
        TelemetryProviders::configure_from_yaml_str(&registry, yaml_hist_explicit).unwrap();
    assert!(providers.meter_provider().is_some());

    // 6. Combined nulls all together
    let yaml_combined_nulls = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        interval: null
        exporter:
          console:
            temporality_preference: null
            default_histogram_aggregation: null
"#;
    let providers =
        TelemetryProviders::configure_from_yaml_str(&registry, yaml_combined_nulls).unwrap();
    assert!(providers.meter_provider().is_some());
}

#[test]
fn test_validation_console_and_interval_negative_cases() {
    let registry = ConfigurationProviderRegistry::default();

    // 1. console: scalar string (disallowed: must be object or null)
    let yaml_scalar_console = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          console: "not-an-object"
"#;
    let err =
        TelemetryProviders::configure_from_yaml_str(&registry, yaml_scalar_console).unwrap_err();
    assert!(err
        .to_string()
        .contains("Field 'console' must be an object or null"));

    // 2. console: scalar number
    let yaml_number_console = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          console: 42
"#;
    let err =
        TelemetryProviders::configure_from_yaml_str(&registry, yaml_number_console).unwrap_err();
    assert!(err
        .to_string()
        .contains("Field 'console' must be an object or null"));

    // 3. console.temporality_preference: unsupported string
    let yaml_bad_temp = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          console:
            temporality_preference: "unsupported"
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_bad_temp).unwrap_err();
    assert!(err
        .to_string()
        .contains("Unsupported temporality preference 'unsupported'"));

    // 4. console.temporality_preference: non-string number
    let yaml_num_temp = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          console:
            temporality_preference: 123
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_num_temp).unwrap_err();
    assert!(err
        .to_string()
        .contains("`console.temporality_preference` must be a string"));

    // 5. console.default_histogram_aggregation: base2_exponential_bucket_histogram is unsupported in this milestone
    let yaml_exp_hist = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          console:
            default_histogram_aggregation: "base2_exponential_bucket_histogram"
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_exp_hist).unwrap_err();
    assert!(err
        .to_string()
        .contains("value 'base2_exponential_bucket_histogram' is not supported in this milestone"));

    // 6. console.default_histogram_aggregation: unrecognized string
    let yaml_bad_hist = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          console:
            default_histogram_aggregation: "unknown_histogram"
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_bad_hist).unwrap_err();
    assert!(err.to_string().contains(
        "value 'unknown_histogram' is not supported; only 'explicit_bucket_histogram' is supported"
    ));

    // 7. console.default_histogram_aggregation: non-string number
    let yaml_num_hist = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          console:
            default_histogram_aggregation: 42
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_num_hist).unwrap_err();
    assert!(err
        .to_string()
        .contains("`console.default_histogram_aggregation` must be a string"));

    // 8. console: unknown field
    let yaml_unknown_console_field = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          console:
            unknown_option: "fail"
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_unknown_console_field)
        .unwrap_err();
    assert!(err
        .to_string()
        .contains("Unknown field 'unknown_option' in console exporter configuration"));

    // 9. periodic.interval: negative number
    let yaml_neg_interval = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        interval: -10
        exporter:
          console: {}
"#;
    let err =
        TelemetryProviders::configure_from_yaml_str(&registry, yaml_neg_interval).unwrap_err();
    assert!(err
        .to_string()
        .contains("`periodic.interval` must be non-negative"));

    // 10. periodic.interval: string value
    let yaml_str_interval = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        interval: "60000"
        exporter:
          console: {}
"#;
    let err =
        TelemetryProviders::configure_from_yaml_str(&registry, yaml_str_interval).unwrap_err();
    assert!(err
        .to_string()
        .contains("`periodic.interval` must be a non-negative integer"));

    // 11. periodic.interval: float value
    let yaml_float_interval = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        interval: 1.5
        exporter:
          console: {}
"#;
    let err =
        TelemetryProviders::configure_from_yaml_str(&registry, yaml_float_interval).unwrap_err();
    assert!(err
        .to_string()
        .contains("`periodic.interval` must be an integer"));
}

#[test]
fn test_validation_null_cannot_bypass_unsupported_fields() {
    let registry = ConfigurationProviderRegistry::default();

    // tracer_provider: null
    let yaml = r#"
file_format: "1.2"
tracer_provider: null
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err
        .to_string()
        .contains("`tracer_provider` is not supported"));

    // logger_provider: null
    let yaml = r#"
file_format: "1.2"
logger_provider: null
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err
        .to_string()
        .contains("`logger_provider` is not supported"));

    // metrics: null
    let yaml = r#"
file_format: "1.2"
metrics: null
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err
        .to_string()
        .contains("Old 'metrics' configuration format is not supported"));

    // views: null
    let yaml = r#"
file_format: "1.2"
meter_provider:
  views: null
  readers:
    - periodic:
        exporter:
          console: {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err
        .to_string()
        .contains("`meter_provider.views` is not supported"));

    // exemplar_filter: null
    let yaml = r#"
file_format: "1.2"
meter_provider:
  exemplar_filter: null
  readers:
    - periodic:
        exporter:
          console: {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err
        .to_string()
        .contains("`meter_provider.exemplar_filter` is not supported"));

    // periodic.timeout: null
    let yaml = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        timeout: null
        exporter:
          console: {}
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err
        .to_string()
        .contains("periodic reader timeout is not supported"));

    // resource.detection: null
    let yaml = r#"
file_format: "1.2"
resource:
  detection/development: null
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err
        .to_string()
        .contains("`resource.detection` is not supported"));

    // resource.schema_url: null
    let yaml = r#"
file_format: "1.2"
resource:
  schema_url: null
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err
        .to_string()
        .contains("`resource.schema_url` is not supported"));

    // resource.attributes_list: null
    let yaml = r#"
file_format: "1.2"
resource:
  attributes_list: null
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err
        .to_string()
        .contains("`resource.attributes_list` is not supported"));

    // pull reader: null
    let yaml = r#"
file_format: "1.2"
meter_provider:
  readers:
    - pull: null
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(err.to_string().contains("Pull readers are not supported"));
}

#[test]
fn test_validation_custom_push_exporter_values() {
    let mut registry = ConfigurationProviderRegistry::default();
    registry.register_metric_exporter_factory(
        "custom_mock",
        move |builder: MeterProviderBuilder, _config: &str| Ok(builder),
    );

    // Valid: custom exporter with object
    let yaml_obj = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          custom_mock: {}
"#;
    assert!(TelemetryProviders::configure_from_yaml_str(&registry, yaml_obj).is_ok());

    // Valid: custom exporter with explicit null per v1.2 PushMetricExporter extension rule
    let yaml_null = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          custom_mock: null
"#;
    assert!(TelemetryProviders::configure_from_yaml_str(&registry, yaml_null).is_ok());

    // Negative: scalar string
    let yaml_string = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          custom_mock: "invalid_scalar"
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_string).unwrap_err();
    assert!(err
        .to_string()
        .contains("Custom exporter 'custom_mock' configuration must be an object or null"));

    // Negative: scalar integer
    let yaml_int = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          custom_mock: 12345
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_int).unwrap_err();
    assert!(err
        .to_string()
        .contains("Custom exporter 'custom_mock' configuration must be an object or null"));

    // Negative: scalar boolean
    let yaml_bool = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          custom_mock: true
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_bool).unwrap_err();
    assert!(err
        .to_string()
        .contains("Custom exporter 'custom_mock' configuration must be an object or null"));

    // Negative: array sequence
    let yaml_seq = r#"
file_format: "1.2"
meter_provider:
  readers:
    - periodic:
        exporter:
          custom_mock: [1, 2, 3]
"#;
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml_seq).unwrap_err();
    assert!(err
        .to_string()
        .contains("Custom exporter 'custom_mock' configuration must be an object or null"));
}

fn assert_invalid_config(yaml: &str, expected_message: &str) {
    let registry = ConfigurationProviderRegistry::default();
    let err = TelemetryProviders::configure_from_yaml_str(&registry, yaml).unwrap_err();
    assert!(
        err.to_string().contains(expected_message),
        "expected error containing {expected_message:?}, got {err}"
    );
}

#[test]
fn test_validation_rejects_unsupported_configuration_fields() {
    let cases = [
        ("propagator: {}", "`propagator` is not supported"),
        ("disabled: true", "`disabled` is not supported"),
        ("attribute_limits: {}", "`attribute_limits` is not supported"),
        ("log_level: info", "`log_level` is not supported"),
        (
            "meter_provider:\n  readers: []\n  meter_configurator/development: {}",
            "`meter_provider.meter_configurator` is not supported",
        ),
        (
            "meter_provider:\n  readers: []\n  view_matching_mode/development: strict",
            "`meter_provider.view_matching_mode` is not supported",
        ),
        (
            "meter_provider:\n  readers:\n    - periodic:\n        exporter: {console: {}}\n        producers: []",
            "`periodic.producers` is not supported",
        ),
        (
            "meter_provider:\n  readers:\n    - periodic:\n        exporter: {console: {}}\n        cardinality_limits: {}",
            "`periodic.cardinality_limits` is not supported",
        ),
        (
            "meter_provider:\n  readers:\n    - periodic:\n        exporter: {console: {}}\n        max_export_batch_size/development: 10",
            "`periodic.max_export_batch_size` is not supported",
        ),
    ];

    for (fields, expected_message) in cases {
        let yaml = format!("file_format: '1.2'\n{fields}\n");
        assert_invalid_config(&yaml, expected_message);
    }
}

#[test]
fn test_validation_rejects_malformed_configuration_shapes() {
    let cases = [
        ("[]", "Configuration root must be an object"),
        (
            "file_format: null",
            "Field 'file_format' must be a string, but got null",
        ),
        ("file_format: true", "Field 'file_format' must be a string"),
        (
            "file_format: '1.2'\nunknown_field: value",
            "Unknown field 'unknown_field' in configuration",
        ),
        (
            "{file_format: '1.2', 1: value}",
            "Configuration keys must be strings",
        ),
        ("file_format: '1.2'\nresource: 1", "Field 'resource' must be an object"),
        (
            "file_format: '1.2'\nresource: {1: value}",
            "Resource keys must be strings",
        ),
        (
            "file_format: '1.2'\nresource:\n  attributes: {}",
            "Field 'resource.attributes' must be an array",
        ),
        (
            "file_format: '1.2'\nresource:\n  attributes: []",
            "`resource.attributes` must not be empty",
        ),
        (
            "file_format: '1.2'\nresource:\n  attributes: [null]",
            "Resource attribute must be an object, but got null",
        ),
        (
            "file_format: '1.2'\nresource:\n  attributes: [scalar]",
            "Resource attribute must be an object",
        ),
        (
            "file_format: '1.2'\nmeter_provider: 1",
            "Field 'meter_provider' must be an object",
        ),
        (
            "file_format: '1.2'\nmeter_provider: {1: value}",
            "MeterProvider keys must be strings",
        ),
        (
            "file_format: '1.2'\nmeter_provider: {unknown: value}",
            "Unknown field 'meter_provider.unknown'",
        ),
        (
            "file_format: '1.2'\nmeter_provider:\n  readers: {}",
            "Field 'meter_provider.readers' must be an array",
        ),
        (
            "file_format: '1.2'\nmeter_provider:\n  readers: [null]",
            "Reader must be an object, but got null",
        ),
        (
            "file_format: '1.2'\nmeter_provider:\n  readers: [scalar]",
            "Reader must be an object",
        ),
        (
            "file_format: '1.2'\nmeter_provider:\n  readers:\n    - {1: {}}",
            "Reader key must be a string",
        ),
        (
            "file_format: '1.2'\nmeter_provider:\n  readers:\n    - unknown: {}",
            "Unknown reader type 'unknown'",
        ),
        (
            "file_format: '1.2'\nmeter_provider:\n  readers:\n    - periodic: []",
            "Field 'periodic' must be an object",
        ),
        (
            "file_format: '1.2'\nmeter_provider:\n  readers:\n    - periodic:\n        exporter: {console: {}}\n        mystery: true",
            "Unknown field 'periodic.mystery'",
        ),
        (
            "file_format: '1.2'\nmeter_provider:\n  readers:\n    - periodic:\n        exporter: scalar",
            "`exporter` in periodic reader must be an object",
        ),
        (
            "file_format: '1.2'\nmeter_provider:\n  readers:\n    - periodic:\n        exporter: {1: {}}",
            "Exporter name must be a string",
        ),
        (
            "file_format: '1.2'\nmeter_provider:\n  readers:\n    - periodic:\n        exporter:\n          console: {1: value}",
            "Console configuration keys must be strings",
        ),
    ];

    for (yaml, expected_message) in cases {
        assert_invalid_config(yaml, expected_message);
    }
}

#[test]
fn test_validation_rejects_invalid_resource_attribute_types() {
    for (attribute, expected_message) in [
        (
            "name: '  '\n      value: ignored",
            "Resource attribute name must not be empty",
        ),
        (
            "name: amount\n      value: invalid\n      type: double",
            "declared as type 'double' but value is not a double",
        ),
        (
            "name: amount\n      value: 1\n      type: custom",
            "has unsupported type 'custom'",
        ),
    ] {
        let yaml = format!("file_format: '1.2'\nresource:\n  attributes:\n    - {attribute}\n");
        assert_invalid_config(&yaml, expected_message);
    }
}
