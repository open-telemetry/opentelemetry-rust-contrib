use opentelemetry::KeyValue;
use opentelemetry_sdk::resource::ResourceDetector;
use opentelemetry_sdk::Resource;
use opentelemetry_semantic_conventions as semconv;
use std::env;
use std::time::Duration;

// For a complete list of reserved environment variables in Lambda, see:
// https://docs.aws.amazon.com/lambda/latest/dg/configuration-envvars.html
const AWS_LAMBDA_FUNCTION_NAME_ENV_VAR: &str = "AWS_LAMBDA_FUNCTION_NAME";
const AWS_REGION_ENV_VAR: &str = "AWS_REGION";
const AWS_LAMBDA_FUNCTION_VERSION_ENV_VAR: &str = "AWS_LAMBDA_FUNCTION_VERSION";
const AWS_LAMBDA_LOG_STREAM_NAME_ENV_VAR: &str = "AWS_LAMBDA_LOG_STREAM_NAME";
const AWS_LAMBDA_MEMORY_LIMIT_ENV_VAR: &str = "AWS_LAMBDA_FUNCTION_MEMORY_SIZE";

/// Resource detector that collects resource information from AWS Lambda environment.
pub struct LambdaResourceDetector;

impl ResourceDetector for LambdaResourceDetector {
    fn detect(&self, _: Duration) -> Resource {
        let lambda_name = env::var(AWS_LAMBDA_FUNCTION_NAME_ENV_VAR).unwrap_or_default();
        // If no lambda name is provided, it means that
        // we're not on a Lambda environment, so we return empty resource.
        if lambda_name.is_empty() {
            return Resource::empty();
        }

        let aws_region = env::var(AWS_REGION_ENV_VAR).unwrap_or_default();
        let function_version = env::var(AWS_LAMBDA_FUNCTION_VERSION_ENV_VAR).unwrap_or_default();
        // Convert memory limit from MB to Bytes as required by semantic conventions.
        let function_memory_limit = env::var(AWS_LAMBDA_MEMORY_LIMIT_ENV_VAR)
            .map(|s| s.parse::<i64>().unwrap_or_default() * 1024 * 1024)
            .unwrap_or_default();
        // Instance attributes corresponds to the log stream name for AWS Lambda;
        // See the FaaS resource specification for more details.
        let instance = env::var(AWS_LAMBDA_LOG_STREAM_NAME_ENV_VAR).unwrap_or_default();

        let attributes = [
            KeyValue::new(semconv::resource::CLOUD_PROVIDER, "aws"),
            KeyValue::new(semconv::resource::CLOUD_REGION, aws_region),
            KeyValue::new(semconv::resource::FAAS_INSTANCE, instance),
            KeyValue::new(semconv::resource::FAAS_NAME, lambda_name),
            KeyValue::new(semconv::resource::FAAS_VERSION, function_version),
            KeyValue::new(semconv::resource::FAAS_MAX_MEMORY, function_memory_limit),
        ];

        Resource::new(attributes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sealed_test::prelude::*;
    use std::env::{remove_var, set_var};

    #[sealed_test]
    fn test_aws_lambda_detector() {
        set_var(AWS_LAMBDA_FUNCTION_NAME_ENV_VAR, "my-lambda-function");
        set_var(AWS_REGION_ENV_VAR, "eu-west-3");
        set_var(AWS_LAMBDA_FUNCTION_VERSION_ENV_VAR, "$LATEST");
        set_var(
            AWS_LAMBDA_LOG_STREAM_NAME_ENV_VAR,
            "2023/01/01/[$LATEST]5d1edb9e525d486696cf01a3503487bc",
        );
        set_var(AWS_LAMBDA_MEMORY_LIMIT_ENV_VAR, "128");

        let expected = Resource::new([
            KeyValue::new(semconv::resource::CLOUD_PROVIDER, "aws"),
            KeyValue::new(semconv::resource::CLOUD_REGION, "eu-west-3"),
            KeyValue::new(
                semconv::resource::FAAS_INSTANCE,
                "2023/01/01/[$LATEST]5d1edb9e525d486696cf01a3503487bc",
            ),
            KeyValue::new(semconv::resource::FAAS_NAME, "my-lambda-function"),
            KeyValue::new(semconv::resource::FAAS_VERSION, "$LATEST"),
            KeyValue::new(semconv::resource::FAAS_MAX_MEMORY, 128),
        ]);

        let detector = LambdaResourceDetector {};
        let got = detector.detect(Duration::from_secs(0));

        assert_eq!(expected, got);

        remove_var(AWS_LAMBDA_FUNCTION_NAME_ENV_VAR);
        remove_var(AWS_REGION_ENV_VAR);
        remove_var(AWS_LAMBDA_FUNCTION_VERSION_ENV_VAR);
        remove_var(AWS_LAMBDA_LOG_STREAM_NAME_ENV_VAR);
        remove_var(AWS_LAMBDA_MEMORY_LIMIT_ENV_VAR);
    }

    #[sealed_test]
    fn test_aws_lambda_detector_returns_empty_if_no_lambda_environment() {
        let detector = LambdaResourceDetector {};
        let got = detector.detect(Duration::from_secs(0));
        assert_eq!(Resource::empty(), got);
    }
}
