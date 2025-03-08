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

/// Resource detector that collects resource information from AWS Lambda environment.
pub struct LambdaResourceDetector;

impl ResourceDetector for LambdaResourceDetector {
    fn detect(&self) -> Resource {
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

        let attributes = [
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

        Resource::builder_empty()
            .with_attributes(attributes)
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
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

    #[test]
    fn test_aws_lambda_detector_returns_empty_if_no_lambda_environment() {
        let detector = LambdaResourceDetector {};
        let got = detector.detect();
        assert_eq!(Resource::builder_empty().build(), got);
    }
}
