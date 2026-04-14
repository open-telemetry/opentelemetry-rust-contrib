//! Primary attribute processor for X-Ray segment and subsegment document builders.
//!
//! This module implements the immediate attribute processing path for the most common
//! OpenTelemetry semantic convention attributes. When `SegmentTranslator::translate_span()`
//! encounters an attribute, it looks it up in the `DispatchTable`. If this processor's ID (0)
//! is returned, the corresponding handler is invoked immediately on the segment or subsegment
//! builder. This contrasts with deferred processing, where attributes are handled by
//! `ValueBuilder` instances in the `additional_builders` array for more complex computations.

use std::borrow::Cow;

use opentelemetry::Value;

use super::super::AnyDocumentBuilder;
use super::{get_bool, get_cow, get_integer, get_string_vec, semconv, SpanAttributeProcessor};

/// Dispatches method calls to either Segment or Subsegment builders.
///
/// This macro eliminates boilerplate when implementing methods that work on both builder types.
/// The `invoke!(res $self, $($method)*)` variant converts `Result<(), Error>` to `bool`
/// (true if Ok, false if Err). The `invoke!($self, $($method)*)` variant dispatches without
/// result handling.
macro_rules! invoke {
    // Variant that converts Result<(), Error> to bool
    (res $self:ident, $($method:tt)*) => {
        {
            match $self {
                AnyDocumentBuilder::Segment(builder) => {
                    match builder.$($method)* {
                     Ok(_) => true,
                     Err(_) => false,
                    }
                }
                AnyDocumentBuilder::Subsegment(builder) => {
                    match builder.$($method)*{
                     Ok(_) => true,
                     Err(_) => false,
                    }
                }
            };
        }
    };
    // Variant that dispatches without result handling
    ($self:ident, $($method:tt)*) => {
        {
            match $self {
                AnyDocumentBuilder::Segment(builder) => {
                    builder.$($method)*;
                }
                AnyDocumentBuilder::Subsegment(builder) => {
                    builder.$($method)*;
                }
            };
        }
    };
}

/// Attribute handler methods for the primary document builder processor.
///
/// These methods correspond to specific OpenTelemetry semantic convention attributes and are
/// invoked by the `SpanAttributeProcessor<35>` implementation. Each method:
/// - Takes an attribute value and attempts to process it
/// - Returns `true` if the attribute was successfully handled, `false` otherwise
/// - May work on both Segment and Subsegment (using `invoke!` macro)
/// - May be Segment-only (EC2, ECS, EKS, Elastic Beanstalk metadata)
/// - May be Subsegment-only (SQL, AWS service metadata)
///
/// This implements the immediate attribute processing path, where attributes are directly
/// applied to the segment/subsegment builder during `translate_span()`, as opposed to
/// deferred processing via `ValueBuilder` instances for complex computations.
impl<'v> AnyDocumentBuilder<'v> {
    /// Handles `http.request.method` attribute for both Segment and Subsegment.
    fn http_request_method(&mut self, value: &'v Value) -> bool {
        invoke!(res self, http().request.method(get_cow(value)));
        true
    }
    /// Handles `client.address` attribute for client IP (both Segment and Subsegment).
    fn http_request_client_ip(&mut self, value: &'v Value) -> bool {
        invoke!(res self, http().request.client_ip(get_cow(value)));
        true
    }
    /// Handles `telemetry.sdk.version` attribute for X-Ray SDK version (both Segment and Subsegment).
    fn aws_xray_sdk_version(&mut self, value: &'v Value) -> bool {
        invoke!(self, aws().xray().sdk_version(get_cow(value)));
        true
    }
    /// Handles `telemetry.auto.version` attribute to mark auto-instrumentation (both Segment and Subsegment).
    fn aws_xray_auto_instrumentation(&mut self, _value: &'v Value) -> bool {
        invoke!(self, aws().xray().auto_instrumentation());
        false
    }
    /// Handles `user_agent.original` attribute (both Segment and Subsegment).
    fn http_request_user_agent(&mut self, value: &'v Value) -> bool {
        invoke!(res self, http().request.user_agent(get_cow(value)));
        true
    }
    /// Handles `http.response.status_code` attribute (both Segment and Subsegment).
    fn http_response_status(&mut self, value: &'v Value) -> bool {
        match get_integer(value) {
            Some(status) => invoke!(self, http().response.status(status as u16)),
            None => return false,
        };
        true
    }
    /// Handles `cloud.account.id` attribute (both Segment and Subsegment).
    fn aws_account_id(&mut self, value: &'v Value) -> bool {
        invoke!(self, aws().account_id(get_cow(value)));
        true
    }
    /// Handles `aws.log.group.arns` attribute (both Segment and Subsegment).
    fn cloudwatch_logs_arn(&mut self, value: &'v Value) -> bool {
        match get_string_vec(value) {
            Some(arns) => invoke!(self, cloudwatch_logs().arn(arns)),
            None => return false,
        };
        true
    }

    /// Handles `client.address` attribute for X-Forwarded-For header (Segment-only).
    fn http_request_x_forwarded_for(&mut self, _value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(document_builder) => {
                document_builder.http().request.x_forwarded_for();
                true
            }
            AnyDocumentBuilder::Subsegment(_) => false,
        }
    }
    /// Handles `host.id` attribute for EC2 instance ID (Segment-only).
    fn aws_ec2_instance_id(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(document_builder) => {
                document_builder.aws().ec2().instance_id(get_cow(value));
                true
            }
            AnyDocumentBuilder::Subsegment(_) => false,
        }
    }
    /// Handles `cloud.availability_zone` attribute for EC2 (Segment-only).
    fn aws_ec2_availability_zone(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(document_builder) => {
                document_builder
                    .aws()
                    .ec2()
                    .availability_zone(get_cow(value));
                true
            }
            AnyDocumentBuilder::Subsegment(_) => false,
        }
    }
    /// Handles `host.type` attribute for EC2 instance size (Segment-only).
    fn aws_ec2_instance_size(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(document_builder) => {
                document_builder.aws().ec2().instance_size(get_cow(value));
                true
            }
            AnyDocumentBuilder::Subsegment(_) => false,
        }
    }
    /// Handles `host.image.id` attribute for EC2 AMI ID (Segment-only).
    fn aws_ec2_ami_id(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(document_builder) => {
                document_builder.aws().ec2().ami_id(get_cow(value));
                true
            }
            AnyDocumentBuilder::Subsegment(_) => false,
        }
    }
    /// Handles `container.name` attribute for ECS (Segment-only).
    fn aws_ecs_container(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(document_builder) => {
                document_builder.aws().ecs().container(get_cow(value));
                true
            }
            AnyDocumentBuilder::Subsegment(_) => false,
        }
    }
    /// Handles `container.id` attribute for ECS (Segment-only).
    fn aws_ecs_container_id(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(document_builder) => {
                document_builder.aws().ecs().container_id(get_cow(value));
                true
            }
            AnyDocumentBuilder::Subsegment(_) => false,
        }
    }
    /// Handles `aws.ecs.container.arn` attribute (Segment-only).
    fn aws_ecs_container_arn(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(document_builder) => {
                document_builder.aws().ecs().container_arn(get_cow(value));
                true
            }
            AnyDocumentBuilder::Subsegment(_) => false,
        }
    }
    /// Handles `aws.ecs.cluster.arn` attribute (Segment-only).
    fn aws_ecs_cluster_arn(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(document_builder) => {
                document_builder.aws().ecs().cluster_arn(get_cow(value));
                true
            }
            AnyDocumentBuilder::Subsegment(_) => false,
        }
    }
    /// Handles `aws.ecs.task.arn` attribute (Segment-only).
    fn aws_ecs_task_arn(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(document_builder) => {
                document_builder.aws().ecs().task_arn(get_cow(value));
                true
            }
            AnyDocumentBuilder::Subsegment(_) => false,
        }
    }
    /// Handles `aws.ecs.task.family` attribute (Segment-only).
    fn aws_ecs_task_family(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(document_builder) => {
                document_builder.aws().ecs().task_family(get_cow(value));
                true
            }
            AnyDocumentBuilder::Subsegment(_) => false,
        }
    }
    /// Handles `aws.ecs.launchtype` attribute (Segment-only).
    fn aws_ecs_launch_type(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(document_builder) => {
                document_builder.aws().ecs().launch_type(get_cow(value));
                true
            }
            AnyDocumentBuilder::Subsegment(_) => false,
        }
    }
    /// Handles `k8s.cluster.name` attribute for EKS (Segment-only).
    fn aws_eks_cluster_name(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(document_builder) => {
                document_builder.aws().eks().cluster_name(get_cow(value));
                true
            }
            AnyDocumentBuilder::Subsegment(_) => false,
        }
    }
    /// Handles `k8s.pod.name` attribute for EKS (Segment-only).
    fn aws_eks_pod(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(document_builder) => {
                document_builder.aws().eks().pod(get_cow(value));
                true
            }
            AnyDocumentBuilder::Subsegment(_) => false,
        }
    }
    /// Handles `k8s.pod.uid` attribute for EKS container ID (Segment-only).
    fn aws_eks_container_id(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(document_builder) => {
                document_builder.aws().eks().container_id(get_cow(value));
                true
            }
            AnyDocumentBuilder::Subsegment(_) => false,
        }
    }
    /// Handles `service.namespace` attribute for Elastic Beanstalk environment (Segment-only).
    fn aws_eb_environment_name(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(document_builder) => {
                document_builder
                    .aws()
                    .elastic_beanstalk()
                    .environment_name(get_cow(value));
                true
            }
            AnyDocumentBuilder::Subsegment(_) => false,
        }
    }
    /// Handles `service.version` attribute for Elastic Beanstalk version (Segment-only).
    fn aws_service_version(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(document_builder) => {
                document_builder
                    .aws()
                    .elastic_beanstalk()
                    .version_label(get_cow(value));
                document_builder.service().version(get_cow(value));
                true
            }
            AnyDocumentBuilder::Subsegment(_) => false,
        }
    }
    /// Handles `enduser.id` attribute for user identification (Segment-only).
    fn enduser_id(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(document_builder) => {
                document_builder.user(get_cow(value)).is_ok()
            }
            AnyDocumentBuilder::Subsegment(_) => false,
        }
    }

    /// Handles `db.system` attribute for SQL database type (Subsegment-only).
    fn sql_database_type(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(_) => false,
            AnyDocumentBuilder::Subsegment(document_builder) => {
                let sql_database_type = get_cow(value);
                // The SemConv and ADOT documentation pretends that this must be set for DynamoDB,
                // but the only effect it has on X-Ray side is to prevent it to correctly rendering
                // the DynamoDB sub-segment operations.
                if sql_database_type != "aws.dynamodb" {
                    document_builder.sql().database_type(get_cow(value));
                    true
                } else {
                    false
                }
            }
        }
    }
    /// Handles `db.user` attribute for SQL user (Subsegment-only).
    fn sql_user(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(_) => false,
            AnyDocumentBuilder::Subsegment(document_builder) => {
                document_builder.sql().user(get_cow(value)).is_ok()
            }
        }
    }
    /// Handles `db.query.text` attribute for SQL query (Subsegment-only).
    fn sql_sanitized_query(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(_) => false,
            AnyDocumentBuilder::Subsegment(document_builder) => {
                document_builder.sql().sanitized_query(get_cow(value));
                true
            }
        }
    }
    /// Handles `db.connection_string` attribute for SQL connection string (Subsegment-only).
    fn sql_connection_string(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(_) => false,
            AnyDocumentBuilder::Subsegment(document_builder) => {
                document_builder.sql().connection_string(get_cow(value));
                true
            }
        }
    }
    /// Handles `aws.region` or `cloud.region` attribute (Subsegment-only).
    fn aws_region(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(_) => false,
            AnyDocumentBuilder::Subsegment(document_builder) => {
                document_builder.aws().region(get_cow(value));
                true
            }
        }
    }
    /// Handles `aws.request_id` attribute (Subsegment-only).
    fn aws_request_id(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(_) => false,
            AnyDocumentBuilder::Subsegment(document_builder) => {
                document_builder.aws().request_id(get_cow(value));
                true
            }
        }
    }
    /// Handles `aws.sqs.queue_url` or `aws.queue_url` attribute (Subsegment-only).
    fn aws_queue_url(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(_) => false,
            AnyDocumentBuilder::Subsegment(document_builder) => {
                document_builder.aws().queue_url(get_cow(value));
                true
            }
        }
    }
    /// Handles `aws.dynamodb.table_names` attribute, extracting first table name (Subsegment-only).
    fn aws_table_names(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(_) => false,
            AnyDocumentBuilder::Subsegment(document_builder) => {
                if let Some(table_names) = get_string_vec(value) {
                    if !table_names.is_empty() {
                        document_builder
                            .aws()
                            .table_name(Cow::from(table_names.get(0).unwrap()));
                        document_builder.aws().table_names(table_names);
                        true
                    } else {
                        true
                    }
                } else {
                    false
                }
            }
        }
    }
    /// Handles `aws.table_name` attribute (Subsegment-only).
    fn aws_table_name(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(_) => false,
            AnyDocumentBuilder::Subsegment(document_builder) => {
                document_builder.aws().table_name(get_cow(value));
                true
            }
        }
    }

    /// Handles `http.request.traced` attribute (Subsegment-only).
    fn http_request_traced(&mut self, value: &'v Value) -> bool {
        match self {
            AnyDocumentBuilder::Segment(_) => false,
            AnyDocumentBuilder::Subsegment(document_builder) => {
                if get_bool(value).is_some_and(|b| b) {
                    document_builder.http().request.traced();
                    true
                } else {
                    false
                }
            }
        }
    }
}

/// Primary attribute processor implementation for document builders.
///
/// This processor has ID = 0 and is registered first in `SegmentTranslator::new()`, giving it
/// priority in the attribute processing pipeline. It handles 35 semantic convention attributes
/// through a dispatch table that maps attribute keys to handler methods.
///
/// The HANDLERS array contains multiple entries for some attribute keys (e.g., `CLIENT_ADDRESS`
/// appears twice for both `client_request_ip` and `x_forwarded_for`). When an attribute is
/// looked up in the `DispatchTable`, all matching handlers are invoked in sequence until one
/// returns true, allowing fallback behavior for attributes with multiple interpretations.
///
/// During `translate_span()`, when this processor's ID is returned from the dispatch table,
/// the handler is called immediately on the segment/subsegment builder. This contrasts with
/// other processors that defer processing to `ValueBuilder` instances for complex computations.
impl<'v> SpanAttributeProcessor<'v, 42> for AnyDocumentBuilder<'v> {
    const HANDLERS: [(&'static str, fn(&mut Self, &'v Value) -> bool); 42] = [
        // both Segment and Subsegment
        (semconv::HTTP_REQUEST_METHOD, Self::http_request_method),
        (
            #[allow(deprecated)]
            semconv::HTTP_METHOD,
            Self::http_request_method,
        ),
        (semconv::CLIENT_ADDRESS, Self::http_request_client_ip),
        (semconv::TELEMETRY_SDK_VERSION, Self::aws_xray_sdk_version),
        (
            semconv::TELEMETRY_AUTO_VERSION,
            Self::aws_xray_auto_instrumentation,
        ),
        (semconv::USER_AGENT_ORIGINAL, Self::http_request_user_agent),
        (
            #[allow(deprecated)]
            semconv::HTTP_STATUS_CODE,
            Self::http_response_status,
        ),
        (
            semconv::HTTP_RESPONSE_STATUS_CODE,
            Self::http_response_status,
        ),
        (semconv::CLOUD_ACCOUNT_ID, Self::aws_account_id),
        (semconv::AWS_LOG_GROUP_ARNS, Self::cloudwatch_logs_arn),
        // Segment only
        (semconv::CLIENT_ADDRESS, Self::http_request_x_forwarded_for),
        (semconv::HOST_ID, Self::aws_ec2_instance_id),
        (
            semconv::CLOUD_AVAILABILITY_ZONE,
            Self::aws_ec2_availability_zone,
        ),
        (semconv::HOST_TYPE, Self::aws_ec2_instance_size),
        (semconv::HOST_IMAGE_ID, Self::aws_ec2_ami_id),
        (semconv::CONTAINER_NAME, Self::aws_ecs_container),
        (semconv::CONTAINER_ID, Self::aws_ecs_container_id),
        (semconv::AWS_ECS_CONTAINER_ARN, Self::aws_ecs_container_arn),
        (semconv::AWS_ECS_CLUSTER_ARN, Self::aws_ecs_cluster_arn),
        (semconv::AWS_ECS_TASK_ARN, Self::aws_ecs_task_arn),
        (semconv::AWS_ECS_TASK_FAMILY, Self::aws_ecs_task_family),
        (semconv::AWS_ECS_LAUNCHTYPE, Self::aws_ecs_launch_type),
        (semconv::K8S_CLUSTER_NAME, Self::aws_eks_cluster_name),
        (semconv::K8S_POD_NAME, Self::aws_eks_pod),
        (semconv::K8S_POD_UID, Self::aws_eks_container_id),
        (semconv::SERVICE_NAMESPACE, Self::aws_eb_environment_name),
        (semconv::SERVICE_VERSION, Self::aws_service_version),
        (semconv::ENDUSER_ID, Self::enduser_id),
        // Subsegment only
        (
            #[allow(deprecated)]
            semconv::DB_SYSTEM,
            Self::sql_database_type,
        ),
        (semconv::DB_SYSTEM_NAME, Self::sql_database_type),
        (
            #[allow(deprecated)]
            semconv::DB_USER,
            Self::sql_user,
        ),
        (
            #[allow(deprecated)]
            semconv::DB_CONNECTION_STRING,
            Self::sql_connection_string,
        ),
        (
            #[allow(deprecated)]
            semconv::DB_STATEMENT,
            Self::sql_sanitized_query,
        ),
        (semconv::DB_QUERY_TEXT, Self::sql_sanitized_query),
        (semconv::AWS_REGION, Self::aws_region),
        (semconv::CLOUD_REGION, Self::aws_region),
        (semconv::AWS_REQUEST_ID, Self::aws_request_id),
        (semconv::AWS_SQS_QUEUE_URL, Self::aws_queue_url),
        (semconv::AWS_QUEUE_URL, Self::aws_queue_url),
        (semconv::AWS_DYNAMODB_TABLE_NAMES, Self::aws_table_names),
        (semconv::AWS_TABLE_NAME, Self::aws_table_name),
        (semconv::AWS_HTTP_TRACED, Self::http_request_traced),
    ];
}

#[cfg(test)]
mod tests {
    use super::super::super::AnyDocumentBuilder;
    use crate::xray_exporter::types::{Id, SubsegmentDocumentBuilder, TraceId};
    use opentelemetry::{Array, Value};

    /// Finalize a subsegment builder by setting required fields, build it,
    /// and return the JSON string for assertion.
    fn build_subsegment_json(builder: AnyDocumentBuilder<'_>) -> String {
        match builder {
            AnyDocumentBuilder::Subsegment(mut b) => {
                b.name("test-subsegment").unwrap();
                b.id(Id::from(0xABCDu64));
                b.parent_id(Id::from(0x1234u64));
                b.start_time(1_000_000.0);
                b.trace_id(TraceId::new(), true).unwrap();
                let doc = b.build().unwrap();
                doc.to_string()
            }
            _ => panic!("expected Subsegment variant"),
        }
    }

    // Tests for sql_database_type

    #[test]
    fn sql_database_type_skips_dynamodb() {
        // 'aws.dynamodb' should be skipped to prevent X-Ray rendering issues
        let mut builder = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        let value = Value::String("aws.dynamodb".into());
        let result = builder.sql_database_type(&value);
        assert!(!result, "aws.dynamodb should return false (not included)");

        // Verify the sql.database_type field is NOT set in the output
        let json = build_subsegment_json(builder);
        assert!(
            !json.contains("\"database_type\""),
            "database_type should not appear in JSON for aws.dynamodb"
        );
    }

    #[test]
    fn sql_database_type_sets_for_other_db() {
        // Non-DynamoDB db types should be accepted and set the sql.database_type field
        let mut builder = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        let value = Value::String("postgresql".into());
        let result = builder.sql_database_type(&value);
        assert!(result, "postgresql should return true");

        let json = build_subsegment_json(builder);
        assert!(
            json.contains("\"database_type\":\"postgresql\""),
            "database_type should be 'postgresql' in JSON, got: {json}"
        );
    }

    // Tests for aws_table_names

    #[test]
    fn aws_table_names_sets_both_fields() {
        // Valid string array sets both table_name (first element) and table_names (all)
        let mut builder = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        let value = Value::Array(Array::String(vec!["table1".into(), "table2".into()]));
        let result = builder.aws_table_names(&value);
        assert!(result, "non-empty string array should return true");

        let json = build_subsegment_json(builder);
        assert!(
            json.contains("\"table_name\":\"table1\""),
            "table_name should be first element 'table1', got: {json}"
        );
        assert!(
            json.contains("\"table_names\""),
            "table_names should be present in JSON, got: {json}"
        );
    }

    #[test]
    fn aws_table_names_empty_array() {
        // Empty array returns true but doesn't set table_name
        let mut builder = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        let value = Value::Array(Array::String(vec![]));
        let result = builder.aws_table_names(&value);
        assert!(result, "empty string array should return true");

        let json = build_subsegment_json(builder);
        assert!(
            !json.contains("\"table_name\""),
            "table_name should not appear for empty array, got: {json}"
        );
    }

    #[test]
    fn aws_table_names_non_string_array() {
        // Non-string-array value returns false
        let mut builder = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        let value = Value::I64(42);
        let result = builder.aws_table_names(&value);
        assert!(!result, "non-string-array value should return false");
    }

    // Tests for http_request_traced

    #[test]
    fn http_request_traced_true_sets_flag() {
        // Value::Bool(true) should set the traced flag
        let mut builder = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        let value = Value::Bool(true);
        let result = builder.http_request_traced(&value);
        assert!(result, "Bool(true) should return true");

        let json = build_subsegment_json(builder);
        assert!(
            json.contains("\"traced\":true"),
            "traced should be true in JSON, got: {json}"
        );
    }

    #[test]
    fn http_request_traced_false_does_not_set_flag() {
        // Value::Bool(false) should NOT set the traced flag
        let mut builder = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        let value = Value::Bool(false);
        let result = builder.http_request_traced(&value);
        assert!(!result, "Bool(false) should return false");

        let json = build_subsegment_json(builder);
        assert!(
            !json.contains("\"traced\""),
            "traced should not appear in JSON for false, got: {json}"
        );
    }

    #[test]
    fn http_request_traced_non_bool_returns_false() {
        // Non-bool value should return false
        let mut builder = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        let value = Value::String("true".into());
        let result = builder.http_request_traced(&value);
        assert!(!result, "non-bool value should return false");
    }
}
