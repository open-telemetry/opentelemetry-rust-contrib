use opentelemetry::{Array, KeyValue, StringValue, Value};
use opentelemetry_sdk::resource::ResourceDetector;
use opentelemetry_sdk::Resource;
use opentelemetry_semantic_conventions as semconv;
use std::env;
use std::path::Path;

// For a complete list of reserved environment variables in Lambda, see:
// https://docs.aws.amazon.com/lambda/latest/dg/configuration-envvars.html
const AWS_LAMBDA_FUNCTION_NAME_ENV_VAR: &str = "AWS_LAMBDA_FUNCTION_NAME";
const AWS_REGION_ENV_VAR: &str = "AWS_REGION";
const AWS_LAMBDA_FUNCTION_VERSION_ENV_VAR: &str = "AWS_LAMBDA_FUNCTION_VERSION";
const AWS_LAMBDA_LOG_STREAM_NAME_ENV_VAR: &str = "AWS_LAMBDA_LOG_STREAM_NAME";
const AWS_LAMBDA_MEMORY_LIMIT_ENV_VAR: &str = "AWS_LAMBDA_FUNCTION_MEMORY_SIZE";
const AWS_LAMBDA_LOG_GROUP_NAME_ENV_VAR: &str = "AWS_LAMBDA_LOG_GROUP_NAME";
const ACCOUNT_ID_SYMLINK_PATH: &str = "/tmp/.otel-account-id";

/// Resource detector that collects resource information from AWS Lambda environment.
pub struct LambdaResourceDetector;

impl ResourceDetector for LambdaResourceDetector {
    fn detect(&self) -> Resource {
        Self::detect_with_symlink_path(ACCOUNT_ID_SYMLINK_PATH)
    }
}

impl LambdaResourceDetector {
    fn detect_with_symlink_path(symlink_path: impl AsRef<Path>) -> Resource {
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

        if let Ok(account_id) = std::fs::read_link(symlink_path) {
            if let Some(account_id_str) = account_id.to_str() {
                attributes.push(KeyValue::new(
                    semconv::resource::CLOUD_ACCOUNT_ID,
                    account_id_str.to_string(),
                ));
            }
        }

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

    #[sealed_test]
    fn test_aws_lambda_detector_with_account_id_symlink() {
        let symlink_path = std::env::temp_dir().join(".otel-account-id-test");
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

                // Verify cloud.account.id is present
                let account_id = got
                    .iter()
                    .find(|(k, _)| k.as_str() == semconv::resource::CLOUD_ACCOUNT_ID);
                assert!(
                    account_id.is_some(),
                    "cloud.account.id attribute should be present"
                );
                assert_eq!(
                    account_id.unwrap().1.as_str(),
                    "123456789012",
                );
            },
        );

        let _ = std::fs::remove_file(&symlink_path);
    }

    #[sealed_test]
    fn test_aws_lambda_detector_missing_symlink_no_panic() {
        let symlink_path = std::env::temp_dir().join(".otel-account-id-nonexistent");
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

                // Verify cloud.account.id is NOT present
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
