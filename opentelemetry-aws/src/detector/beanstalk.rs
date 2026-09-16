use opentelemetry::KeyValue;
use opentelemetry_sdk::resource::ResourceDetector;
use opentelemetry_sdk::Resource;
use opentelemetry_semantic_conventions::attribute as semco;
use serde::Deserialize;
use std::path::Path;

use super::utils::{info_on_error, opt_kv, warn_on_error};

/// Name reported in internal logs emitted by this detector.
const DETECTOR: &str = "aws_elastic_beanstalk";

/// Path to the X-Ray configuration file written by the Elastic Beanstalk platform.
#[cfg(target_os = "windows")]
const CONF_FILE_PATH: &str = r"C:\Program Files\Amazon\XRay\environment.conf";
#[cfg(not(target_os = "windows"))]
const CONF_FILE_PATH: &str = "/var/elasticbeanstalk/xray/environment.conf";

/// Subset of the Elastic Beanstalk `environment.conf` that this detector reads.
#[derive(Deserialize)]
struct EnvironmentConf {
    deployment_id: Option<serde_json::Value>,
    environment_name: Option<String>,
    version_label: Option<String>,
}

/// Elastic Beanstalk resource detector (`detector-aws-beanstalk` feature).
///
/// Reads the X-Ray `environment.conf` file written by the Elastic Beanstalk
/// platform and returns an OTel [`Resource`] with the following attributes:
///
/// | OTel attribute        | Source                                                     |
/// |-----------------------|------------------------------------------------------------|
/// | `cloud.provider`      | hardcoded `"aws"`                                          |
/// | `cloud.platform`      | hardcoded `"aws_elastic_beanstalk"`                       |
/// | `service.name`        | hardcoded `"aws_elastic_beanstalk"`                       |
/// | `service.instance.id` | `deployment_id` from the config file                       |
/// | `service.namespace`   | `environment_name` from the config file                    |
/// | `service.version`     | `version_label` from the config file                       |
///
/// Values that cannot be found or parsed are skipped.
///
/// # Probing
///
/// The `environment.conf` file only exists on Elastic Beanstalk. If it is
/// missing or cannot be parsed, the environment is assumed not to be Elastic
/// Beanstalk and an empty [`Resource`] is returned.
///
/// # Examples
///
/// ```no_run
/// use opentelemetry_aws::detector::BeanstalkResourceDetector;
/// use opentelemetry_sdk::Resource;
///
/// let resource = Resource::builder()
///     .with_detector(Box::new(BeanstalkResourceDetector))
///     .build();
/// ```
///
/// [`Resource`]: opentelemetry_sdk::Resource
pub struct BeanstalkResourceDetector;

impl ResourceDetector for BeanstalkResourceDetector {
    fn detect(&self) -> Resource {
        Self::detect_with_path(CONF_FILE_PATH)
    }
}

impl BeanstalkResourceDetector {
    /// Reads the Elastic Beanstalk config at `path` and builds the resource.
    fn detect_with_path(path: impl AsRef<Path>) -> Resource {
        // A missing file means we are not running on Elastic Beanstalk.
        let Some(contents) = info_on_error(DETECTOR, std::fs::read_to_string(path)) else {
            return Resource::builder_empty().build();
        };
        let Some(conf) =
            warn_on_error(DETECTOR, serde_json::from_str::<EnvironmentConf>(&contents))
        else {
            return Resource::builder_empty().build();
        };

        let deployment_id = conf.deployment_id.and_then(json_scalar_to_string);
        let attribute_options = [
            Some(KeyValue::new(semco::CLOUD_PROVIDER, "aws")),
            Some(KeyValue::new(
                semco::CLOUD_PLATFORM,
                "aws_elastic_beanstalk",
            )),
            Some(KeyValue::new(semco::SERVICE_NAME, "aws_elastic_beanstalk")),
            opt_kv(semco::SERVICE_INSTANCE_ID, deployment_id),
            opt_kv(semco::SERVICE_NAMESPACE, conf.environment_name),
            opt_kv(semco::SERVICE_VERSION, conf.version_label),
        ];

        Resource::builder_empty()
            .with_attributes(attribute_options.into_iter().flatten())
            .build()
    }
}

fn json_scalar_to_string(value: serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_conf(name: &str, contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn detects_full_config() {
        let path = write_conf(
            "otel-aws-beanstalk-full.conf",
            r#"{"deployment_id": 23, "environment_name": "my-env", "version_label": "v1.2.3"}"#,
        );

        let expected = Resource::builder_empty()
            .with_attributes([
                KeyValue::new(semco::CLOUD_PROVIDER, "aws"),
                KeyValue::new(semco::CLOUD_PLATFORM, "aws_elastic_beanstalk"),
                KeyValue::new(semco::SERVICE_NAME, "aws_elastic_beanstalk"),
                KeyValue::new(semco::SERVICE_INSTANCE_ID, "23"),
                KeyValue::new(semco::SERVICE_NAMESPACE, "my-env"),
                KeyValue::new(semco::SERVICE_VERSION, "v1.2.3"),
            ])
            .build();

        let got = BeanstalkResourceDetector::detect_with_path(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(expected, got);
    }

    #[test]
    fn accepts_string_deployment_id() {
        let path = write_conf(
            "otel-aws-beanstalk-string-id.conf",
            r#"{"deployment_id": "42", "environment_name": "e", "version_label": "v"}"#,
        );

        let got = BeanstalkResourceDetector::detect_with_path(&path);
        let _ = std::fs::remove_file(&path);

        let instance_id = got
            .iter()
            .find(|(k, _)| k.as_str() == semco::SERVICE_INSTANCE_ID);
        assert_eq!(instance_id.unwrap().1.as_str(), "42");
    }

    #[test]
    fn returns_empty_when_file_missing() {
        let path = std::env::temp_dir().join("otel-aws-beanstalk-missing.conf");
        let _ = std::fs::remove_file(&path);

        let got = BeanstalkResourceDetector::detect_with_path(&path);
        assert_eq!(Resource::builder_empty().build(), got);
    }

    #[test]
    fn returns_empty_when_malformed() {
        let path = write_conf("otel-aws-beanstalk-malformed.conf", "not json");

        let got = BeanstalkResourceDetector::detect_with_path(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(Resource::builder_empty().build(), got);
    }

    #[test]
    fn skips_absent_fields() {
        let path = write_conf(
            "otel-aws-beanstalk-partial.conf",
            r#"{"environment_name": "only-env"}"#,
        );

        let got = BeanstalkResourceDetector::detect_with_path(&path);
        let _ = std::fs::remove_file(&path);

        assert!(got
            .iter()
            .any(|(k, _)| k.as_str() == semco::SERVICE_NAMESPACE));
        assert!(got
            .iter()
            .all(|(k, _)| k.as_str() != semco::SERVICE_INSTANCE_ID));
        assert!(got
            .iter()
            .all(|(k, _)| k.as_str() != semco::SERVICE_VERSION));
    }
}
