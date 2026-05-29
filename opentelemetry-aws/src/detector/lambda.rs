use opentelemetry::{Array, KeyValue, StringValue, Value};
use opentelemetry_sdk::resource::ResourceDetector;
use opentelemetry_sdk::Resource;
use opentelemetry_semantic_conventions as semconv;
use std::env;

// For a complete list of reserved environment variables in Lambda, see:
// https://docs.aws.amazon.com/lambda/latest/dg/configuration-envvars.html
const AWS_LAMBDA_FUNCTION_NAME_ENV_VAR: &str = "AWS_LAMBDA_FUNCTION_NAME";
const AWS_REGION_ENV_VAR: &str = "AWS_REGION";
const AWS_LAMBDA_FUNCTION_VERSION_ENV_VAR: &str = "AWS_LAMBDA_FUNCTION_VERSION";
const AWS_LAMBDA_LOG_STREAM_NAME_ENV_VAR: &str = "AWS_LAMBDA_LOG_STREAM_NAME";
const AWS_LAMBDA_MEMORY_LIMIT_ENV_VAR: &str = "AWS_LAMBDA_FUNCTION_MEMORY_SIZE";
const AWS_LAMBDA_LOG_GROUP_NAME_ENV_VAR: &str = "AWS_LAMBDA_LOG_GROUP_NAME";

#[cfg(target_os = "linux")]
const ACCOUNT_ID_SYMLINK_PATH: &str = "/tmp/.otel-aws-account-id";

/// Resource detector that collects resource information from AWS Lambda environment.
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
                        semconv::resource::CLOUD_ACCOUNT_ID,
                        account_id_str.to_string(),
                    ));
                }
            }
        }
        Self::build_resource(extra)
    }

    fn build_resource(extra_attributes: Vec<KeyValue>) -> Resource {
        let lambda_name = env::var(AWS_LAMBDA_FUNCTION_NAME_ENV_VAR).unwrap_or_default();
        // If no lambda name is provided, it means that
        // we're not on a Lambda environment, so we return empty resource.
        if lambda_name.is_empty() {
            return Resource::builder_empty().build();
        }

        let aws_region = env::var(AWS_REGION_ENV_VAR).unwrap_or_default();
        let function_version = env::var(AWS_LAMBDA_FUNCTION_VERSION_ENV_VAR).unwrap_or_default();
        // Convert memory limit from MB (string) to Bytes (int) as required by semantic conventions.
        let function_memory_limit = env::var(AWS_LAMBDA_MEMORY_LIMIT_ENV_VAR)
            .map(|s| s.parse::<i64>().unwrap_or_default() * 1024 * 1024)
            .unwrap_or_default();
        // Instance attributes corresponds to the log stream name for AWS Lambda;
        // See the FaaS resource specification for more details.
        let instance = env::var(AWS_LAMBDA_LOG_STREAM_NAME_ENV_VAR).unwrap_or_default();
        let log_group_name = env::var(AWS_LAMBDA_LOG_GROUP_NAME_ENV_VAR).unwrap_or_default();

        let mut attributes = vec![
            KeyValue::new(semconv::resource::CLOUD_PROVIDER, "aws"),
            KeyValue::new(semconv::resource::CLOUD_REGION, aws_region),
            KeyValue::new(semconv::resource::FAAS_INSTANCE, instance),
            KeyValue::new(semconv::resource::FAAS_NAME, lambda_name),
            KeyValue::new(semconv::resource::FAAS_VERSION, function_version),
            KeyValue::new(semconv::resource::FAAS_MAX_MEMORY, function_memory_limit),
            KeyValue::new(
                semconv::resource::AWS_LOG_GROUP_NAMES,
                Value::Array(Array::from(vec![StringValue::from(log_group_name)])),
            ),
        ];
        attributes.extend(extra_attributes);

        Resource::builder_empty()
            .with_attributes(attributes)
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
                        KeyValue::new(semconv::resource::CLOUD_PROVIDER, "aws"),
                        KeyValue::new(semconv::resource::CLOUD_REGION, "eu-west-3"),
                        KeyValue::new(
                            semconv::resource::FAAS_INSTANCE,
                            "2023/01/01/[$LATEST]5d1edb9e525d486696cf01a3503487bc",
                        ),
                        KeyValue::new(semconv::resource::FAAS_NAME, "my-lambda-function"),
                        KeyValue::new(semconv::resource::FAAS_VERSION, "$LATEST"),
                        KeyValue::new(semconv::resource::FAAS_MAX_MEMORY, 128 * 1024 * 1024),
                        KeyValue::new(
                            semconv::resource::AWS_LOG_GROUP_NAMES,
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
                    .find(|(k, _)| k.as_str() == semconv::resource::CLOUD_ACCOUNT_ID);
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
                    .find(|(k, _)| k.as_str() == semconv::resource::CLOUD_ACCOUNT_ID);
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
                    .find(|(k, _)| k.as_str() == semconv::resource::CLOUD_ACCOUNT_ID);
                assert!(
                    account_id.is_none(),
                    "cloud.account.id attribute should not be present when symlink is missing"
                );
            },
        );
    }
}
