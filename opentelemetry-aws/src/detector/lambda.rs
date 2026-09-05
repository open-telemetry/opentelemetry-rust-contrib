use opentelemetry::KeyValue;
use opentelemetry_sdk::resource::ResourceDetector;
use opentelemetry_sdk::Resource;
use opentelemetry_semantic_conventions::attribute as semco;
use std::env;

use super::utils::{info_on_error, non_empty, opt_kv, opt_kv_array, warn_on_error};

// For a complete list of reserved environment variables in Lambda, see:
// https://docs.aws.amazon.com/lambda/latest/dg/configuration-envvars.html

/// Name reported in internal logs emitted by this detector.
const DETECTOR: &str = "aws_lambda";
/// Environment variable that holds the Lambda function name; also used as the platform probe.
const AWS_LAMBDA_FUNCTION_NAME_ENV_VAR: &str = "AWS_LAMBDA_FUNCTION_NAME";
/// Environment variable that holds the AWS Region the function runs in.
const AWS_REGION_ENV_VAR: &str = "AWS_REGION";
/// Environment variable that holds the function version alias or `$LATEST`.
const AWS_LAMBDA_FUNCTION_VERSION_ENV_VAR: &str = "AWS_LAMBDA_FUNCTION_VERSION";
/// Environment variable that holds the CloudWatch Logs stream name for the invocation.
const AWS_LAMBDA_LOG_STREAM_NAME_ENV_VAR: &str = "AWS_LAMBDA_LOG_STREAM_NAME";
/// Environment variable that holds the function's configured memory limit in megabytes.
const AWS_LAMBDA_MEMORY_LIMIT_ENV_VAR: &str = "AWS_LAMBDA_FUNCTION_MEMORY_SIZE";
/// Environment variable that holds the CloudWatch Logs group name for the function.
const AWS_LAMBDA_LOG_GROUP_NAME_ENV_VAR: &str = "AWS_LAMBDA_LOG_GROUP_NAME";

#[cfg(target_os = "linux")]
const ACCOUNT_ID_SYMLINK_PATH: &str = "/tmp/.otel-aws-account-id";

/// Lambda resource detector (`detector-aws-lambda` feature).
///
/// Reads the AWS Lambda [reserved environment variables] and a known (Linux only) filesystem symlink,
/// and returns an OTel [`Resource`] with the following attributes:
///
/// | OTel attribute        | Source                                                                                     |
/// |-----------------------|--------------------------------------------------------------------------------------------|
/// | `cloud.provider`      | hardcoded `"aws"`                                                                          |
/// | `cloud.platform`      | hardcoded `"aws_lambda"`                                                                   |
/// | `cloud.region`        | `AWS_REGION` environment variable                                                          |
/// | `cloud.account.id`    | symlink target at `/tmp/.otel-aws-account-id` (Linux only), accepted only when exactly 12 ASCII decimal digits  |
/// | `faas.name`           | `AWS_LAMBDA_FUNCTION_NAME` environment variable                                            |
/// | `faas.version`        | `AWS_LAMBDA_FUNCTION_VERSION` environment variable                                         |
/// | `faas.instance`       | `AWS_LAMBDA_LOG_STREAM_NAME` environment variable                                          |
/// | `faas.max_memory`     | `AWS_LAMBDA_FUNCTION_MEMORY_SIZE` environment variable, converted from megabytes to bytes  |
/// | `aws.log.group.names` | `AWS_LAMBDA_LOG_GROUP_NAME` environment variable, wrapped in a one-element array           |
///
/// Values that cannot be found or parsed are skipped.
///
/// # Probing
///
/// If `AWS_LAMBDA_FUNCTION_NAME` is unset the environment is assumed not to be
/// Lambda and an empty [`Resource`] is returned immediately.
///
/// # Examples
///
/// Register the detector with the OpenTelemetry SDK so that Lambda attributes
/// are automatically merged into the global resource:
///
/// ```no_run
/// // Requires a running Lambda environment with AWS_LAMBDA_FUNCTION_NAME set.
/// use opentelemetry_aws::detector::LambdaResourceDetector;
/// use opentelemetry_sdk::Resource;
///
/// let resource = Resource::builder()
///     .with_detector(Box::new(LambdaResourceDetector))
///     .build();
/// ```
///
/// [reserved environment variables]: https://docs.aws.amazon.com/lambda/latest/dg/configuration-envvars.html
/// [`Resource`]: opentelemetry_sdk::Resource
pub struct LambdaResourceDetector;

impl ResourceDetector for LambdaResourceDetector {
    fn detect(&self) -> Resource {
        #[cfg(target_os = "linux")]
        return Self::detect_with_symlink_path(ACCOUNT_ID_SYMLINK_PATH);

        #[cfg(not(target_os = "linux"))]
        Self::build_resource(vec![])
    }
}

impl LambdaResourceDetector {
    /// Reads `cloud.account.id` from the symlink at `symlink_path` and builds
    /// the full Lambda resource. Only compiled on Linux, where Lambda runs.
    #[cfg(target_os = "linux")]
    fn detect_with_symlink_path(symlink_path: impl AsRef<std::path::Path>) -> Resource {
        let mut extra = vec![];
        if let Ok(account_id) = std::fs::read_link(symlink_path) {
            if let Some(account_id_str) = account_id.to_str() {
                // Validate that the symlink target looks like a real AWS account ID:
                // exactly 12 ASCII decimal digits. Reject corrupted/garbage targets.
                if account_id_str.len() == 12 && account_id_str.chars().all(|c| c.is_ascii_digit())
                {
                    extra.push(KeyValue::new(
                        semco::CLOUD_ACCOUNT_ID,
                        account_id_str.to_string(),
                    ));
                }
            }
        }
        Self::build_resource(extra)
    }

    fn build_resource(extra_attributes: Vec<KeyValue>) -> Resource {
        // Platform probe: AWS_LAMBDA_FUNCTION_NAME is always set by the Lambda
        // runtime; its absence means we are not running on Lambda.
        let Some(lambda_name) =
            info_on_error(DETECTOR, env::var(AWS_LAMBDA_FUNCTION_NAME_ENV_VAR)).and_then(non_empty)
        else {
            return Resource::builder_empty().build();
        };

        // Convert memory limit from MB (string) to bytes (i64) as required by
        // semantic conventions.
        let function_memory_limit =
            warn_on_error(DETECTOR, env::var(AWS_LAMBDA_MEMORY_LIMIT_ENV_VAR))
                .and_then(|s| warn_on_error(DETECTOR, s.parse::<i64>()))
                .map(|mb| KeyValue::new(semco::FAAS_MAX_MEMORY, mb * 1024 * 1024));

        // aws.log.group.names is typed as string[] by semantic conventions;
        // Lambda exposes a single group name, so it is wrapped in a one-element array.
        let log_group_names = warn_on_error(DETECTOR, env::var(AWS_LAMBDA_LOG_GROUP_NAME_ENV_VAR))
            .and_then(non_empty);

        let attribute_options = [
            Some(KeyValue::new(semco::CLOUD_PROVIDER, "aws")),
            Some(KeyValue::new(semco::CLOUD_PLATFORM, "aws_lambda")),
            Some(KeyValue::new(semco::FAAS_NAME, lambda_name)),
            opt_kv(
                semco::CLOUD_REGION,
                warn_on_error(DETECTOR, env::var(AWS_REGION_ENV_VAR)),
            ),
            opt_kv(
                semco::FAAS_VERSION,
                warn_on_error(DETECTOR, env::var(AWS_LAMBDA_FUNCTION_VERSION_ENV_VAR)),
            ),
            // Instance attribute corresponds to the log stream name for AWS Lambda;
            // See https://opentelemetry.io/docs/specs/semconv/faas/faas-spans/ for more details.
            opt_kv(
                semco::FAAS_INSTANCE,
                warn_on_error(DETECTOR, env::var(AWS_LAMBDA_LOG_STREAM_NAME_ENV_VAR)),
            ),
            function_memory_limit,
            opt_kv_array(semco::AWS_LOG_GROUP_NAMES, log_group_names),
        ];

        Resource::builder_empty()
            .with_attributes(attribute_options.into_iter().flatten())
            .with_attributes(extra_attributes)
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::{Array, StringValue, Value};
    use sealed_test::prelude::*;

    #[sealed_test]
    fn test_aws_lambda_detector() {
        temp_env::with_vars(
            [
                (AWS_LAMBDA_FUNCTION_NAME_ENV_VAR, Some("my-lambda-function")),
                (AWS_REGION_ENV_VAR, Some("eu-west-3")),
                (AWS_LAMBDA_FUNCTION_VERSION_ENV_VAR, Some("$LATEST")),
                (
                    AWS_LAMBDA_LOG_STREAM_NAME_ENV_VAR,
                    Some("2023/01/01/[$LATEST]5d1edb9e525d486696cf01a3503487bc"),
                ),
                (AWS_LAMBDA_MEMORY_LIMIT_ENV_VAR, Some("128")),
                (
                    AWS_LAMBDA_LOG_GROUP_NAME_ENV_VAR,
                    Some("/aws/lambda/my-lambda-function"),
                ),
            ],
            || {
                let expected = Resource::builder_empty()
                    .with_attributes([
                        KeyValue::new(semco::CLOUD_PROVIDER, "aws"),
                        KeyValue::new(semco::CLOUD_PLATFORM, "aws_lambda"),
                        KeyValue::new(semco::CLOUD_REGION, "eu-west-3"),
                        KeyValue::new(
                            semco::FAAS_INSTANCE,
                            "2023/01/01/[$LATEST]5d1edb9e525d486696cf01a3503487bc",
                        ),
                        KeyValue::new(semco::FAAS_NAME, "my-lambda-function"),
                        KeyValue::new(semco::FAAS_VERSION, "$LATEST"),
                        KeyValue::new(semco::FAAS_MAX_MEMORY, 128 * 1024 * 1024),
                        KeyValue::new(
                            semco::AWS_LOG_GROUP_NAMES,
                            Value::Array(Array::from(vec![StringValue::from(
                                "/aws/lambda/my-lambda-function".to_string(),
                            )])),
                        ),
                    ])
                    .build();

                let detector = LambdaResourceDetector {};
                let got = detector.detect();

                assert_eq!(expected, got);
            },
        );
    }

    #[sealed_test]
    fn test_aws_lambda_detector_returns_empty_if_no_lambda_environment() {
        let detector = LambdaResourceDetector {};
        let got = detector.detect();
        assert_eq!(Resource::builder_empty().build(), got);
    }

    #[cfg(target_os = "linux")]
    #[sealed_test]
    fn test_aws_lambda_detector_with_account_id_symlink() {
        let symlink_path = std::env::temp_dir().join(".otel-aws-account-id-test");
        // Clean up any leftover from a previous test run
        let _ = std::fs::remove_file(&symlink_path);
        std::os::unix::fs::symlink("123456789012", &symlink_path).unwrap();

        temp_env::with_vars(
            [
                (AWS_LAMBDA_FUNCTION_NAME_ENV_VAR, Some("my-lambda-function")),
                (AWS_REGION_ENV_VAR, Some("eu-west-3")),
                (AWS_LAMBDA_FUNCTION_VERSION_ENV_VAR, Some("$LATEST")),
                (
                    AWS_LAMBDA_LOG_STREAM_NAME_ENV_VAR,
                    Some("2023/01/01/[$LATEST]5d1edb9e525d486696cf01a3503487bc"),
                ),
                (AWS_LAMBDA_MEMORY_LIMIT_ENV_VAR, Some("128")),
                (
                    AWS_LAMBDA_LOG_GROUP_NAME_ENV_VAR,
                    Some("/aws/lambda/my-lambda-function"),
                ),
            ],
            || {
                let got = LambdaResourceDetector::detect_with_symlink_path(&symlink_path);

                let account_id = got
                    .iter()
                    .find(|(k, _)| k.as_str() == semco::CLOUD_ACCOUNT_ID);
                assert!(
                    account_id.is_some(),
                    "cloud.account.id attribute should be present"
                );
                assert_eq!(account_id.unwrap().1.as_str(), "123456789012");
            },
        );

        let _ = std::fs::remove_file(&symlink_path);
    }

    #[cfg(target_os = "linux")]
    #[sealed_test]
    fn test_aws_lambda_detector_with_corrupted_symlink_target() {
        let symlink_path = std::env::temp_dir().join(".otel-aws-account-id-corrupted-test");
        // Clean up any leftover from a previous test run
        let _ = std::fs::remove_file(&symlink_path);
        // Symlink target is garbage — not a 12-digit account ID
        std::os::unix::fs::symlink("not-an-account-id!!", &symlink_path).unwrap();

        temp_env::with_vars(
            [
                (AWS_LAMBDA_FUNCTION_NAME_ENV_VAR, Some("my-lambda-function")),
                (AWS_REGION_ENV_VAR, Some("eu-west-3")),
                (AWS_LAMBDA_FUNCTION_VERSION_ENV_VAR, Some("$LATEST")),
                (
                    AWS_LAMBDA_LOG_STREAM_NAME_ENV_VAR,
                    Some("2023/01/01/[$LATEST]5d1edb9e525d486696cf01a3503487bc"),
                ),
                (AWS_LAMBDA_MEMORY_LIMIT_ENV_VAR, Some("128")),
                (
                    AWS_LAMBDA_LOG_GROUP_NAME_ENV_VAR,
                    Some("/aws/lambda/my-lambda-function"),
                ),
            ],
            || {
                let got = LambdaResourceDetector::detect_with_symlink_path(&symlink_path);

                let account_id = got
                    .iter()
                    .find(|(k, _)| k.as_str() == semco::CLOUD_ACCOUNT_ID);
                assert!(
                    account_id.is_none(),
                    "cloud.account.id should not be set for a corrupted symlink target"
                );
            },
        );

        let _ = std::fs::remove_file(&symlink_path);
    }

    #[cfg(target_os = "linux")]
    #[sealed_test]
    fn test_aws_lambda_detector_missing_symlink_no_panic() {
        let symlink_path = std::env::temp_dir().join(".otel-aws-account-id-nonexistent");
        // Ensure the symlink does not exist
        let _ = std::fs::remove_file(&symlink_path);

        temp_env::with_vars(
            [
                (AWS_LAMBDA_FUNCTION_NAME_ENV_VAR, Some("my-lambda-function")),
                (AWS_REGION_ENV_VAR, Some("eu-west-3")),
                (AWS_LAMBDA_FUNCTION_VERSION_ENV_VAR, Some("$LATEST")),
                (
                    AWS_LAMBDA_LOG_STREAM_NAME_ENV_VAR,
                    Some("2023/01/01/[$LATEST]5d1edb9e525d486696cf01a3503487bc"),
                ),
                (AWS_LAMBDA_MEMORY_LIMIT_ENV_VAR, Some("128")),
                (
                    AWS_LAMBDA_LOG_GROUP_NAME_ENV_VAR,
                    Some("/aws/lambda/my-lambda-function"),
                ),
            ],
            || {
                let got = LambdaResourceDetector::detect_with_symlink_path(&symlink_path);

                let account_id = got
                    .iter()
                    .find(|(k, _)| k.as_str() == semco::CLOUD_ACCOUNT_ID);
                assert!(
                    account_id.is_none(),
                    "cloud.account.id attribute should not be present when symlink is missing"
                );
            },
        );
    }
}
