use std::borrow::Cow;

use opentelemetry::KeyValue;
use opentelemetry_sdk::{resource::ResourceDetector, Resource};
use opentelemetry_semantic_conventions::attribute as semco;

use ureq::{Agent as HttpClient, Error as HttpClientError};

use thiserror::Error;

use super::{
    imds::{ImdsClient, ImdsError, ImdsProvider},
    utils::{blocking_client, info_on_error, opt_kv, opt_kv_array, warn_on_error},
};

/// Name reported in internal logs emitted by this detector.
const DETECTOR: &str = "aws_ecs";
/// Environment variable providing the base URI for the ECS metadata service.
const ECS_CONTAINER_METADATA_URI_ENV_VAR: &str = "ECS_CONTAINER_METADATA_URI_V4";
/// HTTP request timeout in seconds for ECS metadata calls.
const ECS_METADATA_TIMEOUT_SECS: u64 = 1;
/// Log driver name identifying containers that ship to CloudWatch Logs.
const AWSLOGS_DRIVER: &str = "awslogs";

/// ECS resource detector (`detector-aws-ecs` feature).
///
/// Queries the ECS container metadata endpoint (v4) provided by the ECS agent
/// via the `ECS_CONTAINER_METADATA_URI_V4` environment variable and returns an
/// OTel [`Resource`] with the following attributes:
///
/// | OTel attribute            | Source                                                                      |
/// |---------------------------|-----------------------------------------------------------------------------|
/// | `cloud.provider`          | hardcoded `"aws"`                                                           |
/// | `cloud.platform`          | hardcoded `"aws_ecs"`                                                       |
/// | `cloud.region`            | task ARN region segment                                                     |
/// | `cloud.account.id`        | task ARN account-id segment                                                 |
/// | `cloud.availability_zone` | task metadata `AvailabilityZone` on Fargate; falls back to the identity document `availabilityZone` on the EC2 launch type, via IMDS |
/// | `cloud.resource_id`       | container metadata `ContainerARN`                                           |
/// | `aws.ecs.cluster.arn`     | task metadata `Cluster`, expanded to an ARN when it is a bare cluster name  |
/// | `aws.ecs.task.arn`        | task metadata `TaskARN`                                                     |
/// | `aws.ecs.task.id`         | last segment of the task ARN resource part                                  |
/// | `aws.ecs.task.family`     | task metadata `Family`                                                      |
/// | `aws.ecs.task.revision`   | task metadata `Revision`                                                    |
/// | `aws.ecs.launchtype`      | task metadata `LaunchType`, lowercased (`ec2` or `fargate`)                 |
/// | `aws.ecs.container.arn`   | container metadata `ContainerARN`                                           |
/// | `container.id`            | container metadata `DockerId`                                               |
/// | `container.name`          | container metadata `Name`                                                   |
/// | `container.image.name`    | container metadata `Image`, without the tag and digest                      |
/// | `container.image.tags`    | tag parsed from container metadata `Image`                                  |
/// | `container.image.repo_digests` | container metadata `Image`, when it pins a digest                      |
/// | `container.image.id`      | container metadata `ImageID`                                                |
/// | `aws.log.group.names`     | `LogOptions.awslogs-group`, when `LogDriver` is `awslogs`                   |
/// | `aws.log.group.arns`      | built from the log group, partition, account and CloudWatch region          |
/// | `aws.log.stream.names`    | `LogOptions.awslogs-stream`, when `LogDriver` is `awslogs`                  |
/// | `aws.log.stream.arns`     | built from the log group, log stream, partition, account and region         |
/// | `host.id`                 | identity document `instanceId` (EC2 launch type only, via IMDS)             |
/// | `host.type`               | identity document `instanceType` (EC2 launch type only, via IMDS)           |
/// | `host.image.id`           | identity document `imageId` (EC2 launch type only, via IMDS)                |
/// | `host.arch`               | identity document `architecture`, mapped to `host.arch` values (EC2 launch type only, via IMDS) |
/// | `host.name`               | `/latest/meta-data/hostname` (EC2 launch type only, via IMDS)               |
///
/// Values that cannot be found are skipped.
///
/// # Feature flag
///
/// This type is only available when the `detector-aws-ecs` Cargo feature is
/// enabled.
///
/// # Behavior
///
/// Detection is best-effort. Any metadata request that fails (network error,
/// HTTP error, or JSON parse error) is silently skipped and the corresponding
/// attribute is omitted from the returned [`Resource`].
///
/// If `ECS_CONTAINER_METADATA_URI_V4` is unset the environment is assumed not to
/// be ECS and an empty [`Resource`] is returned, so that `cloud.provider` and
/// `cloud.platform` are never asserted off-platform.
///
/// The ARN-derived attributes all come from the task ARN. When it is absent or
/// malformed, `cloud.region`, `cloud.account.id`, `aws.ecs.task.id`, the
/// `aws.log.*.arns` and a synthesised `aws.ecs.cluster.arn` are all omitted.
///
/// On the EC2 launch type, the task metadata endpoint carries neither
/// `host.*` nor `AvailabilityZone`, so they are read from the EC2 instance's IMDSv2
/// endpoint instead. IMDS is unreachable on Fargate, so it is only probed
/// once the EC2 launch type is confirmed from the task metadata, and all of
/// these attributes are omitted otherwise.
///
/// # Blocking
///
/// Detection performs blocking HTTP requests, each capped by a one second
/// timeout, and will therefore stall the calling thread.
///
/// # Examples
///
/// Register the detector with the OpenTelemetry SDK so that ECS attributes are
/// automatically merged into the global resource:
///
/// ```no_run
/// // Requires a running ECS task with ECS_CONTAINER_METADATA_URI_V4 set.
/// use opentelemetry_aws::detector::EcsResourceDetector;
/// use opentelemetry_sdk::Resource;
///
/// let resource = Resource::builder()
///     .with_detector(Box::new(EcsResourceDetector))
///     .build();
/// ```
///
/// [`Resource`]: opentelemetry_sdk::Resource
pub struct EcsResourceDetector;

impl ResourceDetector for EcsResourceDetector {
    fn detect(&self) -> Resource {
        Self::detect_from(EcsMetadataClient::new(), ImdsClient::new)
    }
}

impl EcsResourceDetector {
    fn detect_from<E: EcsMetadataProvider, I: ImdsProvider>(
        ecs: Result<E, EcsMetadataError>,
        imds: impl FnOnce() -> Result<I, ImdsError>,
    ) -> Resource {
        // Platform probe: the metadata URI is injected by the ECS agent, so its
        // absence is the normal case off-platform and only worth a debug event.
        let Some(ecs_metadata) = info_on_error(DETECTOR, ecs) else {
            // Not ECS, return empty resource
            return Resource::builder_empty().build();
        };

        // Past this point the environment is known to be ECS, so anything that
        // still fails is unexpected and reported as a warning.
        let task = warn_on_error(DETECTOR, ecs_metadata.get_task_metadata());
        // Partition, region and account ID of the task ARN, from which every
        // other ARN is derived. Parsed before `task` is consumed below.
        let arn = task
            .as_ref()
            .and_then(|task| task.task_arn.as_deref())
            .and_then(Arn::parse);
        // The task metadata endpoint only reports `AvailabilityZone` on the
        // Fargate launch type, so the EC2 launch type is detected here to
        // decide whether IMDS should be probed to fill the gap. Borrowed
        // ahead of the closure below, which consumes `task`.
        let is_ec2_launch_type = task
            .as_ref()
            .and_then(|task| task.launch_type.as_deref())
            .is_some_and(|launch_type| launch_type.eq_ignore_ascii_case("ec2"));

        let task_attributes = task
            .map(|task| {
                [
                    opt_kv(
                        semco::AWS_ECS_CLUSTER_ARN,
                        cluster_arn(task.cluster, arn.as_ref()),
                    ),
                    opt_kv(semco::AWS_ECS_TASK_ARN, task.task_arn),
                    opt_kv(semco::AWS_ECS_TASK_FAMILY, task.family),
                    opt_kv(semco::AWS_ECS_TASK_REVISION, task.revision),
                    opt_kv(semco::CLOUD_AVAILABILITY_ZONE, task.availability_zone),
                    // Semantic conventions spell the launch type in lower case
                    opt_kv(
                        semco::AWS_ECS_LAUNCHTYPE,
                        task.launch_type.map(|v| v.to_ascii_lowercase()),
                    ),
                ]
            })
            .unwrap_or_default();

        let container = warn_on_error(DETECTOR, ecs_metadata.get_container_metadata());
        let container_attributes = container
            .map(|container| {
                let image = ImageReference::parse(container.image);
                // aws.log.* only applies to the awslogs driver; the other drivers
                // do not ship to CloudWatch Logs.
                let uses_awslogs = container.log_driver.as_deref() == Some(AWSLOGS_DRIVER);
                let logs = container
                    .log_options
                    .filter(|_| uses_awslogs)
                    .unwrap_or_default();
                // CloudWatch may be targeted in a region other than the task's own.
                let logs_region = logs
                    .region
                    .as_deref()
                    .or_else(|| arn.as_ref().map(|arn| arn.region.as_str()));

                [
                    opt_kv(
                        semco::AWS_ECS_CONTAINER_ARN,
                        container.container_arn.clone(),
                    ),
                    opt_kv(semco::CLOUD_RESOURCE_ID, container.container_arn),
                    opt_kv(semco::CONTAINER_ID, container.docker_id),
                    opt_kv(semco::CONTAINER_NAME, container.name),
                    opt_kv(semco::CONTAINER_IMAGE_NAME, image.name),
                    opt_kv_array(semco::CONTAINER_IMAGE_TAGS, image.tag),
                    opt_kv_array(semco::CONTAINER_IMAGE_REPO_DIGESTS, image.repo_digest),
                    opt_kv(semco::CONTAINER_IMAGE_ID, container.image_id),
                    opt_kv_array(semco::AWS_LOG_GROUP_NAMES, logs.group.clone()),
                    opt_kv_array(
                        semco::AWS_LOG_GROUP_ARNS,
                        log_group_arn(logs.group.as_deref(), logs_region, arn.as_ref()),
                    ),
                    opt_kv_array(semco::AWS_LOG_STREAM_NAMES, logs.stream.clone()),
                    opt_kv_array(
                        semco::AWS_LOG_STREAM_ARNS,
                        log_stream_arn(
                            logs.group.as_deref(),
                            logs.stream.as_deref(),
                            logs_region,
                            arn.as_ref(),
                        ),
                    ),
                ]
            })
            .unwrap_or_default();

        let arn_attributes = arn
            .map(|arn| {
                let task_id = arn.task_id();
                let Arn {
                    region, account_id, ..
                } = arn;
                [
                    opt_kv(semco::CLOUD_REGION, Some(region)),
                    opt_kv(semco::CLOUD_ACCOUNT_ID, Some(account_id)),
                    opt_kv(semco::AWS_ECS_TASK_ID, task_id),
                ]
            })
            .unwrap_or_default();

        // On the EC2 launch type, the ECS task metadata endpoint carries none
        // of `host.*`, and no `AvailabilityZone` either, but the task runs on
        // a real EC2 instance whose own metadata service can supply both.
        // IMDS is unreachable on Fargate, so it is only probed once the EC2
        // launch type is confirmed, to avoid a guaranteed 1s stall otherwise.
        let ec2_host_attributes = is_ec2_launch_type
            .then(|| warn_on_error(DETECTOR, imds()))
            .flatten()
            .and_then(|imds| {
                let document = warn_on_error(DETECTOR, imds.get_identity_document())?;
                Some((imds, document))
            })
            .map(|(imds, document)| {
                [
                    document
                        .host_arch()
                        .map(|arch| KeyValue::new(semco::HOST_ARCH, arch)),
                    opt_kv(semco::CLOUD_AVAILABILITY_ZONE, document.availability_zone),
                    opt_kv(semco::HOST_ID, document.instance_id),
                    opt_kv(semco::HOST_TYPE, document.instance_type),
                    opt_kv(semco::HOST_IMAGE_ID, document.image_id),
                    opt_kv(
                        semco::HOST_NAME,
                        warn_on_error(DETECTOR, imds.get("hostname")),
                    ),
                ]
            })
            .unwrap_or_default();

        let attribute_options = [
            Some(KeyValue::new(semco::CLOUD_PROVIDER, "aws")),
            Some(KeyValue::new(semco::CLOUD_PLATFORM, "aws_ecs")),
        ];

        Resource::builder_empty()
            .with_attributes(attribute_options.into_iter().flatten())
            .with_attributes(task_attributes.into_iter().flatten())
            .with_attributes(container_attributes.into_iter().flatten())
            .with_attributes(arn_attributes.into_iter().flatten())
            .with_attributes(ec2_host_attributes.into_iter().flatten())
            .build()
    }
}

/// The partition, region, account ID and resource parts of an
/// `arn:<partition>:<service>:<region>:<account-id>:<resource>` value.
///
/// Fields are accessed by name rather than by positional index so that callers
/// cannot silently swap the region and account-id segments.
struct Arn {
    partition: String,
    region: String,
    account_id: String,
    resource: String,
}

impl Arn {
    /// Parses an ARN, returning `None` unless it is well formed and carries a
    /// partition, a region and an account ID.
    ///
    /// The resource part is kept verbatim, including any `:` it contains.
    fn parse(arn: &str) -> Option<Self> {
        let mut segments = arn.splitn(6, ':');
        if segments.next()? != "arn" {
            return None;
        }
        let partition = segments.next()?;
        let _service = segments.next()?;
        let region = segments.next()?;
        let account_id = segments.next()?;
        let resource = segments.next()?;

        (!partition.is_empty() && !region.is_empty() && !account_id.is_empty()).then(|| Self {
            partition: partition.to_owned(),
            region: region.to_owned(),
            account_id: account_id.to_owned(),
            resource: resource.to_owned(),
        })
    }

    /// Extracts the task ID from the resource part, which is `task/<id>` or
    /// `task/<cluster-name>/<id>` depending on the ARN format in use.
    fn task_id(&self) -> Option<String> {
        self.resource
            .rsplit('/')
            .next()
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
    }
}

/// Resolves `aws.ecs.cluster.arn` from the task metadata `Cluster` field.
///
/// The metadata endpoint reports `Cluster` as a full ARN on current agents but
/// as a bare cluster name on older ones, so a bare name is expanded using the
/// partition, region and account ID of the task ARN.
fn cluster_arn(cluster: Option<String>, task_arn: Option<&Arn>) -> Option<String> {
    let cluster = cluster.filter(|cluster| !cluster.is_empty())?;
    if cluster.starts_with("arn:") {
        return Some(cluster);
    }
    let task_arn = task_arn?;
    Some(format!(
        "arn:{}:ecs:{}:{}:cluster/{cluster}",
        task_arn.partition, task_arn.region, task_arn.account_id
    ))
}

/// Builds the CloudWatch Logs ARN of the log group.
fn log_group_arn(
    group: Option<&str>,
    region: Option<&str>,
    task_arn: Option<&Arn>,
) -> Option<String> {
    let (group, region, task_arn) = (group?, region?, task_arn?);
    Some(format!(
        "arn:{}:logs:{region}:{}:log-group:{group}:*",
        task_arn.partition, task_arn.account_id
    ))
}

/// Builds the CloudWatch Logs ARN of the log stream.
fn log_stream_arn(
    group: Option<&str>,
    stream: Option<&str>,
    region: Option<&str>,
    task_arn: Option<&Arn>,
) -> Option<String> {
    let (group, stream, region, task_arn) = (group?, stream?, region?, task_arn?);
    Some(format!(
        "arn:{}:logs:{region}:{}:log-group:{group}:log-stream:{stream}",
        task_arn.partition, task_arn.account_id
    ))
}

/// The name, tag and repository digest of a container image reference.
#[derive(Default)]
struct ImageReference {
    name: Option<String>,
    tag: Option<String>,
    /// The whole reference, kept verbatim, when it pins a digest. This is the
    /// form `container.image.repo_digests` expects.
    repo_digest: Option<String>,
}

impl ImageReference {
    /// Splits an image reference into its name, optional tag and optional
    /// repository digest.
    ///
    /// A task definition may pin an image by tag (`repo:tag`), by digest
    /// (`repo@sha256:…`), or by both (`repo:tag@sha256:…`), so the digest is
    /// stripped first — otherwise its own `:` would be mistaken for a tag
    /// separator. In what remains, a `:` only separates a tag when it appears
    /// after the last `/`, so that the port of a registry host such as
    /// `registry:5000/repo` is not mistaken for one either.
    fn parse(image: Option<String>) -> Self {
        let Some(image) = image else {
            return Self::default();
        };
        let (remainder, repo_digest) = match image.rsplit_once('@') {
            Some((remainder, _)) => (remainder.to_owned(), Some(image.clone())),
            None => (image, None),
        };
        let (name, tag) = match remainder.rsplit_once(':') {
            Some((name, tag)) if !tag.contains('/') => (name.to_owned(), Some(tag.to_owned())),
            _ => (remainder, None),
        };
        Self {
            name: Some(name),
            tag,
            repo_digest,
        }
    }
}

/// Errors that can arise interacting with the ECS Metadata service,
/// mostly for display purposes.
#[derive(Debug, Error)]
enum EcsMetadataError {
    #[error(
        "ECS Metadata URI environment variable {ECS_CONTAINER_METADATA_URI_ENV_VAR} not found"
    )]
    NoMetadataUriEnvVar,
    #[error("Could not GET {url}: {error}")]
    GetRequest {
        url: String,
        #[source]
        error: HttpClientError,
    },
    #[error("Could not read JSON response: {0}")]
    JsonResponseRead(#[source] HttpClientError),
}

/// Abstraction over the ECS metadata client, used to inject fakes in tests.
trait EcsMetadataProvider {
    fn get_task_metadata(&self) -> Result<EcsTaskMetadata, EcsMetadataError>;
    fn get_container_metadata(&self) -> Result<EcsContainerMetadata, EcsMetadataError>;
}

/// HTTP client and ECS metadata base URI read from the environment.
struct EcsMetadataClient {
    client: HttpClient,
    metadata_uri: String,
}

impl EcsMetadataClient {
    /// Builds a blocking HTTP client and reads the metadata base URI from `ECS_CONTAINER_METADATA_URI_V4`, erroring if unset.
    fn new() -> Result<Self, EcsMetadataError> {
        let client = blocking_client(std::time::Duration::from_secs(ECS_METADATA_TIMEOUT_SECS));

        let metadata_uri = std::env::var(ECS_CONTAINER_METADATA_URI_ENV_VAR)
            .map_err(|_| EcsMetadataError::NoMetadataUriEnvVar)?;

        Ok(Self {
            client,
            metadata_uri,
        })
    }

    /// GETs `metadata_uri` optionally joined with `path` and deserializes the JSON body.
    fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: Option<&str>,
    ) -> Result<T, EcsMetadataError> {
        let url: Cow<str> = match path {
            Some(p) => Cow::Owned(format!("{}/{p}", self.metadata_uri)),
            None => Cow::Borrowed(&self.metadata_uri),
        };
        self.client
            .get(url.as_ref())
            .call()
            .map_err(|error| EcsMetadataError::GetRequest {
                url: url.into_owned(),
                error,
            })?
            .body_mut()
            .read_json()
            .map_err(EcsMetadataError::JsonResponseRead)
    }
}

impl EcsMetadataProvider for EcsMetadataClient {
    /// Fetches the root endpoint (current container metadata).
    fn get_container_metadata(&self) -> Result<EcsContainerMetadata, EcsMetadataError> {
        self.get_json(None)
    }
    /// Fetches the `/task` endpoint (task metadata).
    fn get_task_metadata(&self) -> Result<EcsTaskMetadata, EcsMetadataError> {
        self.get_json(Some("task"))
    }
}

/// Deserialization target for the ECS container metadata endpoint (root path).
#[derive(Default, serde::Deserialize)]
struct EcsContainerMetadata {
    #[serde(rename = "ContainerARN")]
    container_arn: Option<String>,
    #[serde(rename = "DockerId")]
    docker_id: Option<String>,
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "Image")]
    image: Option<String>,
    #[serde(rename = "ImageID")]
    image_id: Option<String>,
    #[serde(rename = "LogDriver")]
    log_driver: Option<String>,
    #[serde(rename = "LogOptions")]
    log_options: Option<EcsLogOptions>,
}

/// Deserialization target for the `LogOptions` object of the container metadata,
/// as populated by the `awslogs` driver.
#[derive(Default, serde::Deserialize)]
struct EcsLogOptions {
    #[serde(rename = "awslogs-group")]
    group: Option<String>,
    #[serde(rename = "awslogs-stream")]
    stream: Option<String>,
    #[serde(rename = "awslogs-region")]
    region: Option<String>,
}

/// Deserialization target for the ECS task metadata endpoint (`/task`).
#[derive(Default, serde::Deserialize)]
struct EcsTaskMetadata {
    #[serde(rename = "Cluster")]
    cluster: Option<String>,
    #[serde(rename = "TaskARN")]
    task_arn: Option<String>,
    #[serde(rename = "Family")]
    family: Option<String>,
    #[serde(rename = "Revision")]
    revision: Option<String>,
    #[serde(rename = "AvailabilityZone")]
    availability_zone: Option<String>,
    #[serde(rename = "LaunchType")]
    launch_type: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detector::imds::{tests::FakeImdsClient, ImdsError};
    use opentelemetry::{Array, StringValue, Value};

    // ── Fake ECS metadata client ──────────────────────────────────────────────

    /// Test-only fake that returns canned ECS metadata responses.
    ///
    /// Both the task and container payloads are stored as JSON `&str` values
    /// and deserialized on each call, exercising the `serde` impl alongside
    /// the detector logic.
    struct FakeEcsMetadataClient {
        task: &'static str,
        container: &'static str,
    }

    impl FakeEcsMetadataClient {
        fn new() -> Self {
            Self {
                task: "",
                container: "",
            }
        }

        fn with_task(mut self, json: &'static str) -> Self {
            self.task = json;
            self
        }

        fn with_container(mut self, json: &'static str) -> Self {
            self.container = json;
            self
        }
    }

    impl EcsMetadataProvider for FakeEcsMetadataClient {
        fn get_task_metadata(&self) -> Result<EcsTaskMetadata, EcsMetadataError> {
            serde_json::from_str(self.task)
                .map_err(HttpClientError::Json)
                .map_err(EcsMetadataError::JsonResponseRead)
        }

        fn get_container_metadata(&self) -> Result<EcsContainerMetadata, EcsMetadataError> {
            serde_json::from_str(self.container)
                .map_err(HttpClientError::Json)
                .map_err(EcsMetadataError::JsonResponseRead)
        }
    }

    // ── JSON fixtures ─────────────────────────────────────────────────────────

    /// Fargate task — cluster already given as a full ARN, AZ present.
    const FARGATE_TASK: &str = r#"
        {
            "Cluster":          "arn:aws:ecs:us-east-1:123456789012:cluster/my-cluster",
            "TaskARN":          "arn:aws:ecs:us-east-1:123456789012:task/my-cluster/abc123def456",
            "Family":           "my-family",
            "Revision":         "3",
            "AvailabilityZone": "us-east-1b",
            "LaunchType":       "FARGATE"
        }
    "#;

    /// EC2 task — cluster given as a bare name (should be expanded to ARN), no AZ.
    const EC2_TASK: &str = r#"
        {
            "Cluster":  "my-cluster",
            "TaskARN":  "arn:aws:ecs:us-east-1:123456789012:task/my-cluster/abc123def456",
            "Family":   "my-family",
            "Revision": "3",
            "LaunchType": "EC2"
        }
    "#;

    /// Task with no TaskARN — ARN-derived attrs (region, account.id, task.id,
    /// cluster ARN expansion) should all be absent.
    const TASK_NO_ARN: &str = r#"
        {
            "Cluster":          "my-cluster",
            "Family":           "my-family",
            "AvailabilityZone": "us-east-1b",
            "LaunchType":       "FARGATE"
        }
    "#;

    /// Container using the `awslogs` log driver — log group / stream attrs expected.
    const CONTAINER_WITH_AWSLOGS: &str = r#"
        {
            "ContainerARN": "arn:aws:ecs:us-east-1:123456789012:container/abc",
            "DockerId":     "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
            "Name":         "my-container",
            "Image":        "nginx:1.21",
            "ImageID":      "sha256:aabbcc",
            "LogDriver":    "awslogs",
            "LogOptions": {
                "awslogs-group":  "/ecs/my-group",
                "awslogs-stream": "my-stream"
            }
        }
    "#;

    /// Container using a non-awslogs driver — log attrs should be absent.
    const CONTAINER_NO_LOGS: &str = r#"
        {
            "ContainerARN": "arn:aws:ecs:us-east-1:123456789012:container/abc",
            "DockerId":     "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
            "Name":         "my-container",
            "Image":        "nginx:1.21",
            "LogDriver":    "json-file",
            "LogOptions": {
                "awslogs-group":  "/ecs/my-group",
                "awslogs-stream": "my-stream"
            }
        }
    "#;

    /// IMDS instance identity document for an EC2-backed ECS task.
    /// account_id / region are not consumed by the ECS detector (it derives
    /// those from the task ARN), but they are present here to keep the fixture
    /// realistic.
    const EC2_IMDS_DOC: &str = r#"
        {
            "accountId":        "123456789012",
            "region":           "us-east-1",
            "availabilityZone": "us-east-1a",
            "instanceId":       "i-0ec2instance",
            "instanceType":     "m5.large",
            "imageId":          "ami-0abcdef",
            "architecture":     "x86_64"
        }
    "#;

    // ── helpers ───────────────────────────────────────────────────────────────

    fn str_array(v: &str) -> Value {
        Value::Array(Array::from(vec![StringValue::from(v.to_string())]))
    }

    // ── Construction failure → empty resource ─────────────────────────────────

    #[test]
    fn detect_from_ecs_construction_failure_returns_empty() {
        let resource = EcsResourceDetector::detect_from::<FakeEcsMetadataClient, FakeImdsClient>(
            Err(EcsMetadataError::NoMetadataUriEnvVar),
            || panic!("IMDS closure must not be called"),
        );
        assert_eq!(resource, Resource::builder_empty().build());
    }

    // ── Fargate: IMDS closure not invoked ─────────────────────────────────────

    #[test]
    fn detect_from_fargate_does_not_invoke_imds() {
        let ecs = FakeEcsMetadataClient::new()
            .with_task(FARGATE_TASK)
            .with_container(CONTAINER_NO_LOGS);

        let imds = || -> Result<FakeImdsClient, _> {
            panic!("IMDS closure must not be called on Fargate")
        };

        let resource = EcsResourceDetector::detect_from(Ok(ecs), imds);

        let expected = Resource::builder_empty()
            .with_attributes([
                KeyValue::new(semco::CLOUD_PROVIDER, "aws"),
                KeyValue::new(semco::CLOUD_PLATFORM, "aws_ecs"),
                KeyValue::new(
                    semco::AWS_ECS_CLUSTER_ARN,
                    "arn:aws:ecs:us-east-1:123456789012:cluster/my-cluster",
                ),
                KeyValue::new(
                    semco::AWS_ECS_TASK_ARN,
                    "arn:aws:ecs:us-east-1:123456789012:task/my-cluster/abc123def456",
                ),
                KeyValue::new(semco::AWS_ECS_TASK_FAMILY, "my-family"),
                KeyValue::new(semco::AWS_ECS_TASK_REVISION, "3"),
                KeyValue::new(semco::CLOUD_AVAILABILITY_ZONE, "us-east-1b"),
                KeyValue::new(semco::AWS_ECS_LAUNCHTYPE, "fargate"),
                KeyValue::new(
                    semco::AWS_ECS_CONTAINER_ARN,
                    "arn:aws:ecs:us-east-1:123456789012:container/abc",
                ),
                KeyValue::new(
                    semco::CLOUD_RESOURCE_ID,
                    "arn:aws:ecs:us-east-1:123456789012:container/abc",
                ),
                KeyValue::new(
                    semco::CONTAINER_ID,
                    "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
                ),
                KeyValue::new(semco::CONTAINER_NAME, "my-container"),
                KeyValue::new(semco::CONTAINER_IMAGE_NAME, "nginx"),
                KeyValue::new(semco::CONTAINER_IMAGE_TAGS, str_array("1.21")),
                KeyValue::new(semco::CLOUD_REGION, "us-east-1"),
                KeyValue::new(semco::CLOUD_ACCOUNT_ID, "123456789012"),
                KeyValue::new(semco::AWS_ECS_TASK_ID, "abc123def456"),
            ])
            .build();

        assert_eq!(resource, expected);
    }

    // ── EC2 launch type: IMDS closure invoked → host.* attributes ─────────────

    #[test]
    fn detect_from_ec2_launch_type_queries_imds() {
        let ecs = FakeEcsMetadataClient::new()
            .with_task(EC2_TASK)
            .with_container(CONTAINER_NO_LOGS);

        let imds = || {
            Ok(FakeImdsClient::new()
                .with_document(EC2_IMDS_DOC)
                .with_get("hostname", "ip-10-0-0-5.ec2.internal"))
        };

        let resource = EcsResourceDetector::detect_from(Ok(ecs), imds);

        let expected = Resource::builder_empty()
            .with_attributes([
                KeyValue::new(semco::CLOUD_PROVIDER, "aws"),
                KeyValue::new(semco::CLOUD_PLATFORM, "aws_ecs"),
                // task attrs
                KeyValue::new(
                    semco::AWS_ECS_CLUSTER_ARN,
                    "arn:aws:ecs:us-east-1:123456789012:cluster/my-cluster",
                ),
                KeyValue::new(
                    semco::AWS_ECS_TASK_ARN,
                    "arn:aws:ecs:us-east-1:123456789012:task/my-cluster/abc123def456",
                ),
                KeyValue::new(semco::AWS_ECS_TASK_FAMILY, "my-family"),
                KeyValue::new(semco::AWS_ECS_TASK_REVISION, "3"),
                KeyValue::new(semco::AWS_ECS_LAUNCHTYPE, "ec2"),
                // container attrs
                KeyValue::new(
                    semco::AWS_ECS_CONTAINER_ARN,
                    "arn:aws:ecs:us-east-1:123456789012:container/abc",
                ),
                KeyValue::new(
                    semco::CLOUD_RESOURCE_ID,
                    "arn:aws:ecs:us-east-1:123456789012:container/abc",
                ),
                KeyValue::new(
                    semco::CONTAINER_ID,
                    "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
                ),
                KeyValue::new(semco::CONTAINER_NAME, "my-container"),
                KeyValue::new(semco::CONTAINER_IMAGE_NAME, "nginx"),
                KeyValue::new(semco::CONTAINER_IMAGE_TAGS, str_array("1.21")),
                // ARN-derived attrs
                KeyValue::new(semco::CLOUD_REGION, "us-east-1"),
                KeyValue::new(semco::CLOUD_ACCOUNT_ID, "123456789012"),
                KeyValue::new(semco::AWS_ECS_TASK_ID, "abc123def456"),
                // IMDS host attrs
                KeyValue::new(semco::HOST_ARCH, "amd64"),
                KeyValue::new(semco::CLOUD_AVAILABILITY_ZONE, "us-east-1a"),
                KeyValue::new(semco::HOST_ID, "i-0ec2instance"),
                KeyValue::new(semco::HOST_TYPE, "m5.large"),
                KeyValue::new(semco::HOST_IMAGE_ID, "ami-0abcdef"),
                KeyValue::new(semco::HOST_NAME, "ip-10-0-0-5.ec2.internal"),
            ])
            .build();

        assert_eq!(resource, expected);
    }

    // ── EC2 launch type: IMDS fails → no host.* ───────────────────────────────

    #[test]
    fn detect_from_ec2_launch_type_imds_failure_omits_host_attrs() {
        let ecs = FakeEcsMetadataClient::new()
            .with_task(EC2_TASK)
            .with_container(CONTAINER_NO_LOGS);

        let imds = || -> Result<FakeImdsClient, _> { Err(ImdsError::EmptyAuthToken) };

        let resource = EcsResourceDetector::detect_from(Ok(ecs), imds);

        let expected = Resource::builder_empty()
            .with_attributes([
                KeyValue::new(semco::CLOUD_PROVIDER, "aws"),
                KeyValue::new(semco::CLOUD_PLATFORM, "aws_ecs"),
                KeyValue::new(
                    semco::AWS_ECS_CLUSTER_ARN,
                    "arn:aws:ecs:us-east-1:123456789012:cluster/my-cluster",
                ),
                KeyValue::new(
                    semco::AWS_ECS_TASK_ARN,
                    "arn:aws:ecs:us-east-1:123456789012:task/my-cluster/abc123def456",
                ),
                KeyValue::new(semco::AWS_ECS_TASK_FAMILY, "my-family"),
                KeyValue::new(semco::AWS_ECS_TASK_REVISION, "3"),
                KeyValue::new(semco::AWS_ECS_LAUNCHTYPE, "ec2"),
                KeyValue::new(
                    semco::AWS_ECS_CONTAINER_ARN,
                    "arn:aws:ecs:us-east-1:123456789012:container/abc",
                ),
                KeyValue::new(
                    semco::CLOUD_RESOURCE_ID,
                    "arn:aws:ecs:us-east-1:123456789012:container/abc",
                ),
                KeyValue::new(
                    semco::CONTAINER_ID,
                    "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
                ),
                KeyValue::new(semco::CONTAINER_NAME, "my-container"),
                KeyValue::new(semco::CONTAINER_IMAGE_NAME, "nginx"),
                KeyValue::new(semco::CONTAINER_IMAGE_TAGS, str_array("1.21")),
                KeyValue::new(semco::CLOUD_REGION, "us-east-1"),
                KeyValue::new(semco::CLOUD_ACCOUNT_ID, "123456789012"),
                KeyValue::new(semco::AWS_ECS_TASK_ID, "abc123def456"),
                // No HOST_* attributes
            ])
            .build();

        assert_eq!(resource, expected);
    }

    // ── awslogs driver → log attrs present ────────────────────────────────────

    #[test]
    fn detect_from_awslogs_driver_produces_log_attrs() {
        let ecs = FakeEcsMetadataClient::new()
            .with_task(FARGATE_TASK)
            .with_container(CONTAINER_WITH_AWSLOGS);

        let imds = || -> Result<FakeImdsClient, _> { panic!("should not be called") };

        let resource = EcsResourceDetector::detect_from(Ok(ecs), imds);

        let expected = Resource::builder_empty()
            .with_attributes([
                KeyValue::new(semco::CLOUD_PROVIDER, "aws"),
                KeyValue::new(semco::CLOUD_PLATFORM, "aws_ecs"),
                KeyValue::new(
                    semco::AWS_ECS_CLUSTER_ARN,
                    "arn:aws:ecs:us-east-1:123456789012:cluster/my-cluster",
                ),
                KeyValue::new(
                    semco::AWS_ECS_TASK_ARN,
                    "arn:aws:ecs:us-east-1:123456789012:task/my-cluster/abc123def456",
                ),
                KeyValue::new(semco::AWS_ECS_TASK_FAMILY, "my-family"),
                KeyValue::new(semco::AWS_ECS_TASK_REVISION, "3"),
                KeyValue::new(semco::CLOUD_AVAILABILITY_ZONE, "us-east-1b"),
                KeyValue::new(semco::AWS_ECS_LAUNCHTYPE, "fargate"),
                KeyValue::new(
                    semco::AWS_ECS_CONTAINER_ARN,
                    "arn:aws:ecs:us-east-1:123456789012:container/abc",
                ),
                KeyValue::new(
                    semco::CLOUD_RESOURCE_ID,
                    "arn:aws:ecs:us-east-1:123456789012:container/abc",
                ),
                KeyValue::new(
                    semco::CONTAINER_ID,
                    "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
                ),
                KeyValue::new(semco::CONTAINER_NAME, "my-container"),
                KeyValue::new(semco::CONTAINER_IMAGE_NAME, "nginx"),
                KeyValue::new(semco::CONTAINER_IMAGE_TAGS, str_array("1.21")),
                KeyValue::new(semco::CONTAINER_IMAGE_ID, "sha256:aabbcc"),
                KeyValue::new(semco::AWS_LOG_GROUP_NAMES, str_array("/ecs/my-group")),
                KeyValue::new(
                    semco::AWS_LOG_GROUP_ARNS,
                    str_array(
                        "arn:aws:logs:us-east-1:123456789012:log-group:/ecs/my-group:*",
                    ),
                ),
                KeyValue::new(semco::AWS_LOG_STREAM_NAMES, str_array("my-stream")),
                KeyValue::new(
                    semco::AWS_LOG_STREAM_ARNS,
                    str_array(
                        "arn:aws:logs:us-east-1:123456789012:log-group:/ecs/my-group:log-stream:my-stream",
                    ),
                ),
                KeyValue::new(semco::CLOUD_REGION, "us-east-1"),
                KeyValue::new(semco::CLOUD_ACCOUNT_ID, "123456789012"),
                KeyValue::new(semco::AWS_ECS_TASK_ID, "abc123def456"),
            ])
            .build();

        assert_eq!(resource, expected);
    }

    // ── non-awslogs driver → log attrs absent ─────────────────────────────────

    #[test]
    fn detect_from_non_awslogs_driver_omits_log_attrs() {
        let ecs = FakeEcsMetadataClient::new()
            .with_task(FARGATE_TASK)
            .with_container(CONTAINER_NO_LOGS);

        let imds = || -> Result<FakeImdsClient, _> { panic!("should not be called") };

        let resource = EcsResourceDetector::detect_from(Ok(ecs), imds);

        let expected = Resource::builder_empty()
            .with_attributes([
                KeyValue::new(semco::CLOUD_PROVIDER, "aws"),
                KeyValue::new(semco::CLOUD_PLATFORM, "aws_ecs"),
                KeyValue::new(
                    semco::AWS_ECS_CLUSTER_ARN,
                    "arn:aws:ecs:us-east-1:123456789012:cluster/my-cluster",
                ),
                KeyValue::new(
                    semco::AWS_ECS_TASK_ARN,
                    "arn:aws:ecs:us-east-1:123456789012:task/my-cluster/abc123def456",
                ),
                KeyValue::new(semco::AWS_ECS_TASK_FAMILY, "my-family"),
                KeyValue::new(semco::AWS_ECS_TASK_REVISION, "3"),
                KeyValue::new(semco::CLOUD_AVAILABILITY_ZONE, "us-east-1b"),
                KeyValue::new(semco::AWS_ECS_LAUNCHTYPE, "fargate"),
                KeyValue::new(
                    semco::AWS_ECS_CONTAINER_ARN,
                    "arn:aws:ecs:us-east-1:123456789012:container/abc",
                ),
                KeyValue::new(
                    semco::CLOUD_RESOURCE_ID,
                    "arn:aws:ecs:us-east-1:123456789012:container/abc",
                ),
                KeyValue::new(
                    semco::CONTAINER_ID,
                    "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
                ),
                KeyValue::new(semco::CONTAINER_NAME, "my-container"),
                KeyValue::new(semco::CONTAINER_IMAGE_NAME, "nginx"),
                KeyValue::new(semco::CONTAINER_IMAGE_TAGS, str_array("1.21")),
                KeyValue::new(semco::CLOUD_REGION, "us-east-1"),
                KeyValue::new(semco::CLOUD_ACCOUNT_ID, "123456789012"),
                KeyValue::new(semco::AWS_ECS_TASK_ID, "abc123def456"),
            ])
            .build();

        assert_eq!(resource, expected);
    }

    // ── Absent task ARN → ARN-derived attrs omitted ───────────────────────────

    #[test]
    fn detect_from_missing_task_arn_omits_arn_derived_attrs() {
        let ecs = FakeEcsMetadataClient::new()
            .with_task(TASK_NO_ARN)
            .with_container("{}");

        let imds = || -> Result<FakeImdsClient, _> { panic!("should not be called") };

        let resource = EcsResourceDetector::detect_from(Ok(ecs), imds);

        let expected = Resource::builder_empty()
            .with_attributes([
                KeyValue::new(semco::CLOUD_PROVIDER, "aws"),
                KeyValue::new(semco::CLOUD_PLATFORM, "aws_ecs"),
                // cluster ARN not synthesised: bare name + no task ARN → omitted
                KeyValue::new(semco::AWS_ECS_TASK_FAMILY, "my-family"),
                KeyValue::new(semco::CLOUD_AVAILABILITY_ZONE, "us-east-1b"),
                KeyValue::new(semco::AWS_ECS_LAUNCHTYPE, "fargate"),
            ])
            .build();

        assert_eq!(resource, expected);
    }

    // ── Arn::parse ────────────────────────────────────────────────────────────

    #[test]
    fn arn_parse_valid_full() {
        let arn = Arn::parse("arn:aws:ecs:us-east-1:123456789012:task/abc").unwrap();
        assert_eq!(arn.partition, "aws");
        assert_eq!(arn.region, "us-east-1");
        assert_eq!(arn.account_id, "123456789012");
        assert_eq!(arn.resource, "task/abc");
    }

    #[test]
    fn arn_parse_resource_with_colons_preserved() {
        // splitn(6, ':') keeps everything after the 5th ':' as one chunk.
        let arn = Arn::parse("arn:aws:logs:us-east-1:123456789123:log-group:my-group:*").unwrap();
        assert_eq!(arn.resource, "log-group:my-group:*");
    }

    #[test]
    fn arn_parse_wrong_prefix() {
        assert!(Arn::parse("xrn:aws:ecs:us-east-1:123456789123:task/abc").is_none());
    }

    #[test]
    fn arn_parse_too_few_segments() {
        // Only 5 colon-separated segments — resource is missing.
        assert!(Arn::parse("arn:aws:ecs:us-east-1:123456789123").is_none());
    }

    #[test]
    fn arn_parse_empty_required_fields() {
        // Empty partition
        assert!(Arn::parse("arn::ecs:us-east-1:123456789123:task/abc").is_none());
        // Empty region
        assert!(Arn::parse("arn:aws:ecs::123456789123:task/abc").is_none());
        // Empty account_id
        assert!(Arn::parse("arn:aws:ecs:us-east-1::task/abc").is_none());
    }

    // ── Arn::task_id ─────────────────────────────────────────────────────────

    fn sample_arn(resource: impl Into<String>) -> Arn {
        Arn {
            partition: "aws".into(),
            region: "us-east-1".into(),
            account_id: "123456789123".into(),
            resource: resource.into(),
        }
    }

    #[test]
    fn task_id_some() {
        // Simple "task/<id>" format
        let arn = sample_arn("task/abcdef");
        assert_eq!(arn.task_id(), Some("abcdef".to_owned()));

        // Long format "task/<cluster>/<id>" — last segment wins
        let arn2 = sample_arn("task/cluster-name/abcdef");
        assert_eq!(arn2.task_id(), Some("abcdef".to_owned()));
    }

    #[test]
    fn task_id_none_on_trailing_slash() {
        let arn = sample_arn("task/");
        assert!(arn.task_id().is_none());
    }

    // ── cluster_arn ───────────────────────────────────────────────────────────

    #[test]
    fn cluster_arn_none_or_empty_cluster() {
        // None cluster
        assert!(cluster_arn(None, Some(&sample_arn("task/abcdef"))).is_none());
        // Empty string cluster
        assert!(cluster_arn(Some(String::new()), Some(&sample_arn("task/abcdef"))).is_none());
        // None regardless of task_arn
        assert!(cluster_arn(None, None).is_none());
    }

    #[test]
    fn cluster_arn_passthrough_when_already_arn() {
        let full = "arn:aws:ecs:us-east-1:123456789123:cluster/my-cluster".to_owned();
        // Returned as-is even when task_arn is None
        assert_eq!(cluster_arn(Some(full.clone()), None), Some(full.clone()));
        assert_eq!(
            cluster_arn(Some(full.clone()), Some(&sample_arn("cluster/my-cluster"))),
            Some(full)
        );
    }

    #[test]
    fn cluster_arn_expands_bare_name_with_task_arn() {
        let result = cluster_arn(
            Some("my-cluster".into()),
            Some(&sample_arn("cluster/my-cluster")),
        );
        assert_eq!(
            result,
            Some("arn:aws:ecs:us-east-1:123456789123:cluster/my-cluster".to_owned())
        );
    }

    #[test]
    fn cluster_arn_bare_name_without_task_arn_is_none() {
        assert!(cluster_arn(Some("my-cluster".into()), None).is_none());
    }

    // ── log_group_arn ─────────────────────────────────────────────────────────

    #[test]
    fn log_group_arn_some() {
        let arn = sample_arn("task/abcdef");
        let result = log_group_arn(Some("my-group"), Some("us-west-2"), Some(&arn));
        assert_eq!(
            result,
            Some("arn:aws:logs:us-west-2:123456789123:log-group:my-group:*".to_owned())
        );
    }

    #[test]
    fn log_group_arn_none_when_any_arg_missing() {
        let arn = sample_arn("task/abcdef");
        // group missing
        assert!(log_group_arn(None, Some("us-west-2"), Some(&arn)).is_none());
        // region missing
        assert!(log_group_arn(Some("my-group"), None, Some(&arn)).is_none());
        // task_arn missing
        assert!(log_group_arn(Some("my-group"), Some("us-west-2"), None).is_none());
    }

    // ── log_stream_arn ────────────────────────────────────────────────────────

    #[test]
    fn log_stream_arn_some() {
        let arn = sample_arn("task/abcdef");
        let result = log_stream_arn(Some("g"), Some("s"), Some("us-west-2"), Some(&arn));
        assert_eq!(
            result,
            Some("arn:aws:logs:us-west-2:123456789123:log-group:g:log-stream:s".to_owned())
        );
    }

    #[test]
    fn log_stream_arn_none_when_any_arg_missing() {
        let arn = sample_arn("task/abcdef");
        // group missing
        assert!(log_stream_arn(None, Some("s"), Some("us-west-2"), Some(&arn)).is_none());
        // stream missing
        assert!(log_stream_arn(Some("g"), None, Some("us-west-2"), Some(&arn)).is_none());
        // region missing
        assert!(log_stream_arn(Some("g"), Some("s"), None, Some(&arn)).is_none());
        // task_arn missing
        assert!(log_stream_arn(Some("g"), Some("s"), Some("us-west-2"), None).is_none());
    }

    // ── ImageReference::parse ─────────────────────────────────────────────────

    #[test]
    fn image_parse_none_input() {
        let img = ImageReference::parse(None);
        assert!(img.name.is_none());
        assert!(img.tag.is_none());
        assert!(img.repo_digest.is_none());
    }

    #[test]
    fn image_parse_plain_repo() {
        let img = ImageReference::parse(Some("myrepo".into()));
        assert_eq!(img.name, Some("myrepo".to_owned()));
        assert!(img.tag.is_none());
        assert!(img.repo_digest.is_none());
    }

    #[test]
    fn image_parse_repo_with_tag() {
        let img = ImageReference::parse(Some("nginx:1.21".into()));
        assert_eq!(img.name, Some("nginx".to_owned()));
        assert_eq!(img.tag, Some("1.21".to_owned()));
        assert!(img.repo_digest.is_none());
    }

    #[test]
    fn image_parse_digest_only() {
        let img = ImageReference::parse(Some("nginx@sha256:abc123".into()));
        assert_eq!(img.repo_digest, Some("nginx@sha256:abc123".to_owned()));
        assert_eq!(img.name, Some("nginx".to_owned()));
        assert!(img.tag.is_none());
    }

    #[test]
    fn image_parse_tag_and_digest() {
        let img = ImageReference::parse(Some("nginx:1.21@sha256:abc123".into()));
        assert_eq!(img.repo_digest, Some("nginx:1.21@sha256:abc123".to_owned()));
        assert_eq!(img.name, Some("nginx".to_owned()));
        assert_eq!(img.tag, Some("1.21".to_owned()));
    }

    #[test]
    fn image_parse_registry_host_with_port_no_tag() {
        // The ':' before '5000' is followed by a path containing '/', so it
        // must NOT be treated as a tag separator.
        let img = ImageReference::parse(Some("registry:5000/repo".into()));
        assert_eq!(img.name, Some("registry:5000/repo".to_owned()));
        assert!(img.tag.is_none());
        assert!(img.repo_digest.is_none());
    }

    #[test]
    fn image_parse_registry_host_with_port_and_tag() {
        // The last ':' separates a tag part "1.0" that contains no '/'.
        let img = ImageReference::parse(Some("registry:5000/repo:1.0".into()));
        assert_eq!(img.name, Some("registry:5000/repo".to_owned()));
        assert_eq!(img.tag, Some("1.0".to_owned()));
        assert!(img.repo_digest.is_none());
    }
}
