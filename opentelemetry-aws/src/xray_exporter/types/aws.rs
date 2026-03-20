use std::{borrow::Cow, marker::PhantomData};

use crate::{field_setter, flag_setter, xray_exporter::types::StrList};
use serde::Serialize;

use super::{
    segment_document::builder_type::{DocumentBuilderType, Segment, Subsegment},
    utils::MaybeSkip,
};

/// AWS-specific information for segments and subsegments.
///
/// For segments, this contains information about the AWS resource on which
/// your application is running. For subsegments, this contains information
/// about the AWS services and resources that your application accessed.
#[derive(Debug, Serialize)]
pub(super) struct AwsData<'a> {
    /// The AWS account ID where the resource is running or being accessed
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    account_id: Option<Cow<'a, str>>,

    /// Information about an Amazon EC2 instance (segments only)
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    ec2: Ec2Metadata<'a>,

    /// Information about an Amazon ECS container (segments only)
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    ecs: EcsMetadata<'a>,

    /// Information about an Amazon EKS pod (segments only)
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    eks: EksMetadata<'a>,

    /// Information about an Elastic Beanstalk environment (segments only)
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    elastic_beanstalk: ElasticBeanstalkMetadata<'a>,

    /// Information about X-Ray SDK instrumentation (segments only)
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    xray: XrayMetadata<'a>,

    /// The name of the API action invoked against an AWS service (subsegments only)
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    operation: Option<Cow<'a, str>>,

    /// The region of the AWS resource if different from your application (subsegments only)
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    region: Option<Cow<'a, str>>,

    /// Unique identifier for the request to an AWS service (subsegments only)
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    request_id: Option<Cow<'a, str>>,

    /// For Amazon SQS operations, the queue's URL (subsegments only)
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    queue_url: Option<Cow<'a, str>>,

    /// For DynamoDB operations, the name of the table (subsegments only)
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    table_name: Option<Cow<'a, str>>,

    /// For DynamoDB operations, the names of the tables (subsegments only)
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    table_names: Option<&'a dyn StrList>,
}

impl MaybeSkip for AwsData<'_> {
    /// Returns true if this AWS data is empty (all fields are None)
    fn skip(&self) -> bool {
        self.account_id.skip()
            && self.ec2.skip()
            && self.ecs.skip()
            && self.eks.skip()
            && self.elastic_beanstalk.skip()
            && self.xray.skip()
            && self.operation.skip()
            && self.region.skip()
            && self.request_id.skip()
            && self.queue_url.skip()
            && self.table_name.skip()
            && self.table_names.skip()
    }
}
/// Builder for constructing AWS-specific metadata.
#[derive(Debug, Default)]
pub(crate) struct AwsDataBuilder<'a, DBT: DocumentBuilderType> {
    account_id: Option<Cow<'a, str>>,
    ec2: Ec2MetadataBuilder<'a>,
    ecs: EcsMetadataBuilder<'a>,
    eks: EksMetadataBuilder<'a>,
    elastic_beanstalk: ElasticBeanstalkMetadataBuilder<'a>,
    xray: XrayMetadataBuilder<'a>,
    operation: Option<Cow<'a, str>>,
    region: Option<Cow<'a, str>>,
    request_id: Option<Cow<'a, str>>,
    queue_url: Option<Cow<'a, str>>,
    table_name: Option<Cow<'a, str>>,
    table_names: Option<&'a dyn StrList>,
    _phatom_data: PhantomData<DBT>,
}

impl<'a, DBT: DocumentBuilderType> AwsDataBuilder<'a, DBT> {
    field_setter!(account_id);

    pub fn xray(&mut self) -> &mut XrayMetadataBuilder<'a> {
        &mut self.xray
    }

    /// Builds the `AwsData` instance.
    pub(super) fn build(self) -> AwsData<'a> {
        AwsData {
            account_id: self.account_id,
            ec2: self.ec2.build(),
            ecs: self.ecs.build(),
            eks: self.eks.build(),
            elastic_beanstalk: self.elastic_beanstalk.build(),
            xray: self.xray.build(),
            operation: self.operation,
            region: self.region,
            request_id: self.request_id,
            queue_url: self.queue_url,
            table_name: self.table_name,
            table_names: self.table_names,
        }
    }
}
impl<'a> AwsDataBuilder<'a, Segment> {
    pub fn ec2(&mut self) -> &mut Ec2MetadataBuilder<'a> {
        &mut self.ec2
    }
    pub fn ecs(&mut self) -> &mut EcsMetadataBuilder<'a> {
        &mut self.ecs
    }
    pub fn eks(&mut self) -> &mut EksMetadataBuilder<'a> {
        &mut self.eks
    }
    pub fn elastic_beanstalk(&mut self) -> &mut ElasticBeanstalkMetadataBuilder<'a> {
        &mut self.elastic_beanstalk
    }
}
impl<'a> AwsDataBuilder<'a, Subsegment> {
    field_setter!(operation);
    field_setter!(region);
    field_setter!(request_id);
    field_setter!(queue_url);
    field_setter!(table_name);
    field_setter!(table_names: &'a dyn StrList);
}

/// Information about an Amazon EC2 instance.
#[derive(Debug, Serialize)]
struct Ec2Metadata<'a> {
    /// The EC2 instance ID
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    instance_id: Option<Cow<'a, str>>,

    /// The type of EC2 instance
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    instance_size: Option<Cow<'a, str>>,

    /// The Amazon Machine Image ID
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    ami_id: Option<Cow<'a, str>>,

    /// The availability zone where the instance is running
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    availability_zone: Option<Cow<'a, str>>,
}

impl MaybeSkip for Ec2Metadata<'_> {
    /// Returns true if this EC2 metadata is empty (all fields are None)
    fn skip(&self) -> bool {
        self.instance_id.skip()
            && self.instance_size.skip()
            && self.ami_id.skip()
            && self.availability_zone.skip()
    }
}

/// Builder for constructing EC2 instance metadata.
#[derive(Debug, Default)]
pub(crate) struct Ec2MetadataBuilder<'a> {
    instance_id: Option<Cow<'a, str>>,
    instance_size: Option<Cow<'a, str>>,
    ami_id: Option<Cow<'a, str>>,
    availability_zone: Option<Cow<'a, str>>,
}

impl<'a> Ec2MetadataBuilder<'a> {
    field_setter!(instance_id);
    field_setter!(instance_size);
    field_setter!(ami_id);
    field_setter!(availability_zone);

    /// Builds the `Ec2Metadata` instance.
    fn build(self) -> Ec2Metadata<'a> {
        Ec2Metadata {
            instance_id: self.instance_id,
            instance_size: self.instance_size,
            ami_id: self.ami_id,
            availability_zone: self.availability_zone,
        }
    }
}

/// Information about an Amazon ECS container.
#[derive(Debug, Serialize)]
struct EcsMetadata<'a> {
    /// The hostname of the container
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    container: Option<Cow<'a, str>>,

    /// The ID of the container
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    container_id: Option<Cow<'a, str>>,

    /// The ARN of the container
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    container_arn: Option<Cow<'a, str>>,

    /// The ARN of the cluster
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    cluster_arn: Option<Cow<'a, str>>,

    /// The ARN of the task
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    task_arn: Option<Cow<'a, str>>,

    /// The task familly
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    task_family: Option<Cow<'a, str>>,

    /// The launchtype
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    launch_type: Option<Cow<'a, str>>,
}

impl MaybeSkip for EcsMetadata<'_> {
    /// Returns true if this ECS metadata is empty (container field is None)
    fn skip(&self) -> bool {
        self.container.skip()
            && self.container_id.skip()
            && self.container_arn.skip()
            && self.cluster_arn.skip()
            && self.task_arn.skip()
            && self.task_family.skip()
            && self.launch_type.skip()
    }
}

/// Builder for constructing ECS container metadata.
#[derive(Debug, Default)]
pub(crate) struct EcsMetadataBuilder<'a> {
    container: Option<Cow<'a, str>>,
    container_id: Option<Cow<'a, str>>,
    container_arn: Option<Cow<'a, str>>,
    cluster_arn: Option<Cow<'a, str>>,
    task_arn: Option<Cow<'a, str>>,
    task_family: Option<Cow<'a, str>>,
    launch_type: Option<Cow<'a, str>>,
}

impl<'a> EcsMetadataBuilder<'a> {
    field_setter!(container);
    field_setter!(container_id);
    field_setter!(container_arn);
    field_setter!(cluster_arn);
    field_setter!(task_arn);
    field_setter!(task_family);
    field_setter!(launch_type);

    /// Builds the `EcsMetadata` instance.
    fn build(self) -> EcsMetadata<'a> {
        EcsMetadata {
            container: self.container,
            container_id: self.container_id,
            container_arn: self.container_arn,
            cluster_arn: self.cluster_arn,
            task_arn: self.task_arn,
            task_family: self.task_family,
            launch_type: self.launch_type,
        }
    }
}

/// Information about an Amazon EKS pod.
#[derive(Debug, Serialize)]
struct EksMetadata<'a> {
    /// The hostname of the container
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    cluster_name: Option<Cow<'a, str>>,

    /// The pod name
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    pod: Option<Cow<'a, str>>,

    /// The ID of the pod
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    container_id: Option<Cow<'a, str>>,
}

impl MaybeSkip for EksMetadata<'_> {
    /// Returns true if this ECS metadata is empty (container field is None)
    fn skip(&self) -> bool {
        self.cluster_name.skip() && self.pod.skip() && self.container_id.skip()
    }
}

/// Builder for constructing EKS pod metadata.
#[derive(Debug, Default)]
pub(crate) struct EksMetadataBuilder<'a> {
    cluster_name: Option<Cow<'a, str>>,
    pod: Option<Cow<'a, str>>,
    container_id: Option<Cow<'a, str>>,
}

impl<'a> EksMetadataBuilder<'a> {
    field_setter!(cluster_name);
    field_setter!(pod);
    field_setter!(container_id);

    /// Builds the `EksMetadata` instance.
    fn build(self) -> EksMetadata<'a> {
        EksMetadata {
            cluster_name: self.cluster_name,
            pod: self.pod,
            container_id: self.container_id,
        }
    }
}

/// Information about an Elastic Beanstalk environment.
#[derive(Debug, Serialize)]
struct ElasticBeanstalkMetadata<'a> {
    /// The name of the Elastic Beanstalk environment
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    environment_name: Option<Cow<'a, str>>,

    /// The version label of the application version currently deployed
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    version_label: Option<Cow<'a, str>>,

    /// The deployment ID of the last successful deployment
    #[serde(skip_serializing_if = "Option::is_none")]
    deployment_id: Option<i64>,
}

impl MaybeSkip for ElasticBeanstalkMetadata<'_> {
    /// Returns true if this Elastic Beanstalk metadata is empty (all fields are None)
    fn skip(&self) -> bool {
        self.environment_name.skip() && self.version_label.skip() && self.deployment_id.is_none()
    }
}

/// Builder for constructing Elastic Beanstalk environment metadata.
#[derive(Debug, Default)]
pub(crate) struct ElasticBeanstalkMetadataBuilder<'a> {
    environment_name: Option<Cow<'a, str>>,
    version_label: Option<Cow<'a, str>>,
    deployment_id: Option<i64>,
}

impl<'a> ElasticBeanstalkMetadataBuilder<'a> {
    field_setter!(environment_name);
    field_setter!(version_label);
    field_setter!(deployment_id:i64);

    /// Builds the `ElasticBeanstalkMetadata` instance.
    fn build(self) -> ElasticBeanstalkMetadata<'a> {
        ElasticBeanstalkMetadata {
            environment_name: self.environment_name,
            version_label: self.version_label,
            deployment_id: self.deployment_id,
        }
    }
}

/// Information about X-Ray SDK instrumentation.
#[derive(Debug, Serialize)]
struct XrayMetadata<'a> {
    /// The SDK name and language
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    sdk: Option<Cow<'a, str>>,
    /// The SDK version string.
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    sdk_version: Option<Cow<'a, str>>,
    /// Whether auto-instrumentation is enabled.
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    auto_instrumentation: bool,
}

impl MaybeSkip for XrayMetadata<'_> {
    /// Returns true if this tracing metadata is empty (sdk field is None)
    fn skip(&self) -> bool {
        self.sdk.skip() && self.sdk_version.skip() && !self.auto_instrumentation
    }
}

/// Builder for constructing X-Ray SDK instrumentation metadata.
#[derive(Debug, Default)]
pub(crate) struct XrayMetadataBuilder<'a> {
    sdk: Option<Cow<'a, str>>,
    sdk_version: Option<Cow<'a, str>>,
    auto_instrumentation: bool,
}

impl<'a> XrayMetadataBuilder<'a> {
    field_setter!(sdk);
    field_setter!(sdk_version);

    flag_setter!(auto_instrumentation);

    /// Builds the `TracingMetadata` instance.
    fn build(self) -> XrayMetadata<'a> {
        XrayMetadata {
            sdk: self.sdk,
            sdk_version: self.sdk_version,
            auto_instrumentation: self.auto_instrumentation,
        }
    }
}
