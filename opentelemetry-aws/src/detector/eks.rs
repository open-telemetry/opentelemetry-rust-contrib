use std::path::Path;

use opentelemetry::KeyValue;
use opentelemetry_sdk::{resource::ResourceDetector, Resource};
use opentelemetry_semantic_conventions::attribute as semco;

use thiserror::Error;

use super::{
    imds::{ImdsClient, ImdsError, ImdsProvider},
    utils::{debug_on_error, info_on_error, non_empty, opt_kv, warn_on_error},
};

/// Name reported in internal logs emitted by this detector.
const DETECTOR: &str = "aws_eks";
/// Filesystem path to the Kubernetes service-account namespace file.
const K8S_NAMESPACE_FILE_PATH: &str = "/var/run/secrets/kubernetes.io/serviceaccount/namespace";
/// Filesystem path of the cgroup membership file used to derive `container.id`.
const CGROUP_FILE_PATH: &str = "/proc/self/cgroup";
/// Filesystem path of the mount table, used as a fallback for `container.id`.
const MOUNTINFO_FILE_PATH: &str = "/proc/self/mountinfo";
/// Minimum length, in hexadecimal characters, of a container ID.
const MIN_CONTAINER_ID_LEN: usize = 32;
/// Maximum length, in hexadecimal characters, of a container ID.
const MAX_CONTAINER_ID_LEN: usize = 64;
/// IMDSv2 instance tag exposing the EKS cluster name, relative to `/latest/meta-data/`.
const EKS_CLUSTER_NAME_TAG_PATH: &str = "tags/instance/aws:eks:cluster-name";
/// The environment variable we expect to contain the cluster name.
const EKS_CLUSTER_NAME_ENV_VAR: &str = "AWS_CLUSTER_NAME";

/// EKS resource detector (`detector-aws-eks` feature).
///
/// Detects an EKS environment by reading the Kubernetes service-account
/// namespace file (`/var/run/secrets/kubernetes.io/serviceaccount/namespace`),
/// querying the EC2 Instance Metadata Service v2 (IMDSv2) at the link-local
/// address `169.254.169.254`, and reading environment variables. It returns an
/// OTel [`Resource`] with the following attributes:
///
/// | OTel attribute            | Source                                                                                        |
/// |---------------------------|-----------------------------------------------------------------------------------------------|
/// | `cloud.provider`          | hardcoded `"aws"`                                                                             |
/// | `cloud.platform`          | hardcoded `"aws_eks"`                                                                         |
/// | `k8s.namespace.name`      | service-account namespace file                                                                |
/// | `k8s.pod.name`            | `HOSTNAME` environment variable                                                               |
/// | `k8s.pod.uid`             | `POD_UID` environment variable (requires the downward API, see note below)                    |
/// | `k8s.node.name`           | `NODE_NAME` environment variable (requires the downward API, see note below)                  |
/// | `k8s.cluster.name`        | IMDSv2 instance tag `aws:eks:cluster-name`; falls back to `AWS_CLUSTER_NAME` (see note below) |
/// | `aws.eks.cluster.arn`     | built from the partition, region, account ID and cluster name                                 |
/// | `container.id`            | container ID parsed from `/proc/self/cgroup`, then from `/proc/self/mountinfo`                |
/// | `cloud.region`            | instance identity document `region`; falls back to `AWS_REGION`                               |
/// | `cloud.account.id`        | instance identity document `accountId`; falls back to `AWS_ACCOUNT_ID`                        |
/// | `cloud.availability_zone` | instance identity document `availabilityZone`                                                 |
/// | `host.id`                 | instance identity document `instanceId` (the node's EC2 instance)                             |
/// | `host.type`               | instance identity document `instanceType` (the node's EC2 instance)                           |
/// | `host.image.id`           | instance identity document `imageId` (the node's EC2 instance)                                |
/// | `host.arch`               | instance identity document `architecture`, mapped to `host.arch` values                       |
/// | `host.name`               | `/latest/meta-data/hostname` (the node's EC2 instance)                                        |
///
/// Values that cannot be found are skipped.
///
/// # Feature flag
///
/// This type is only available when the `detector-aws-eks` Cargo feature is
/// enabled.
///
/// # Behavior
///
/// Detection is best-effort. Any operation that fails (file read error, IMDSv2
/// network error, missing environment variable) is silently skipped and the
/// corresponding attribute is omitted from the returned [`Resource`].
///
/// Detection is gated on two probes, both of which must pass before
/// `cloud.provider` and `cloud.platform` are asserted:
///
/// 1. the service-account namespace file must be readable, which proves the
///    process runs in a Kubernetes pod;
/// 2. something must tie that pod to AWS — either a parsable instance identity
///    document from IMDSv2, or one of the `AWS_CLUSTER_NAME` and `AWS_REGION`
///    environment variables.
///
/// # `k8s.cluster.name` may require configuration
///
/// The cluster name is read from the node's `aws:eks:cluster-name` EC2 instance
/// tag, which is only visible through IMDSv2 when instance tags in metadata are
/// enabled on the node group (`InstanceMetadataTags: enabled`). EKS does not
/// inject `AWS_CLUSTER_NAME` into pod environments, so when the tag is not
/// exposed — including on EKS-on-Fargate, where IMDS is unreachable — you must
/// set `AWS_CLUSTER_NAME` yourself, for instance via a `Deployment` manifest.
/// Setting `OTEL_RESOURCE_ATTRIBUTES=k8s.cluster.name=…` is a portable
/// alternative that does not depend on this detector.
///
/// `aws.eks.cluster.arn` is built from the cluster name, so it inherits the
/// same requirement, and additionally needs the region, the account ID and the
/// partition. The partition is only available from IMDSv2, so the ARN is never
/// reported when IMDS is unreachable, even if the cluster name is configured.
///
/// # `k8s.pod.uid` and `k8s.node.name` require the downward API
///
/// Kubernetes does not expose these to containers by default. Add them to the
/// pod spec to have them detected:
///
/// ```yaml
/// env:
///   - name: POD_UID
///     valueFrom:
///       fieldRef:
///         fieldPath: metadata.uid
///   - name: NODE_NAME
///     valueFrom:
///       fieldRef:
///         fieldPath: spec.nodeName
/// ```
///
/// # Blocking
///
/// Detection performs blocking HTTP requests, each capped by a one second
/// timeout, and will therefore stall the calling thread.
///
/// # Examples
///
/// Register the detector with the OpenTelemetry SDK so that EKS attributes are
/// automatically merged into the global resource:
///
/// ```no_run
/// // Requires a running EKS pod with the service-account token mount present.
/// use opentelemetry_aws::detector::EksResourceDetector;
/// use opentelemetry_sdk::Resource;
///
/// let resource = Resource::builder()
///     .with_detector(Box::new(EksResourceDetector))
///     .build();
/// ```
///
/// [`Resource`]: opentelemetry_sdk::Resource
pub struct EksResourceDetector;

impl ResourceDetector for EksResourceDetector {
    fn detect(&self) -> Resource {
        Self::detect_from(
            ImdsClient::new(),
            Path::new(K8S_NAMESPACE_FILE_PATH),
            Path::new(CGROUP_FILE_PATH),
            Path::new(MOUNTINFO_FILE_PATH),
        )
    }
}

impl EksResourceDetector {
    fn detect_from<P: ImdsProvider>(
        imds: Result<P, ImdsError>,
        namespace_path: &Path,
        cgroup_path: &Path,
        mountinfo_path: &Path,
    ) -> Resource {
        // Kubernetes probe: without the service-account mount, this is not a
        // pod at all. An unreadable file is the normal case off-Kubernetes.
        let Some(namespace) = info_on_error(DETECTOR, get_namespace(namespace_path)) else {
            // Not Kubernetes, return empty resource
            return Resource::builder_empty().build();
        };

        // The node-level attributes come from IMDS, with environment variables as
        // a fallback for the EKS-on-Fargate case where IMDS is unreachable.
        let imds = debug_on_error(DETECTOR, imds);

        let document = imds
            .as_ref()
            .and_then(|imds| warn_on_error(DETECTOR, imds.get_identity_document()));
        let (region, account_id, ec2_document_attributes) = document
            .map(|document| {
                let host_arch = document
                    .host_arch()
                    .map(|arch| KeyValue::new(semco::HOST_ARCH, arch));
                (
                    document.region,
                    document.account_id,
                    [
                        host_arch,
                        opt_kv(semco::CLOUD_AVAILABILITY_ZONE, document.availability_zone),
                        opt_kv(semco::HOST_ID, document.instance_id),
                        opt_kv(semco::HOST_TYPE, document.instance_type),
                        opt_kv(semco::HOST_IMAGE_ID, document.image_id),
                    ],
                )
            })
            .unwrap_or_default();

        // Region and account ID — identity document first, then env var
        let region = region.or_else(|| std::env::var("AWS_REGION").ok());
        let account_id = account_id.or_else(|| std::env::var("AWS_ACCOUNT_ID").ok());

        // Cluster name — from the node's EKS instance tag if exposed through IMDS, then from an env var.
        // The tag is absent unless instance tags in metadata are enabled, which
        // is not the default, so its absence is not worth a warning.
        let cluster_name = imds
            .as_ref()
            .and_then(|imds| debug_on_error(DETECTOR, imds.get(EKS_CLUSTER_NAME_TAG_PATH)))
            .and_then(non_empty)
            .or_else(|| std::env::var(EKS_CLUSTER_NAME_ENV_VAR).ok())
            .ok_or(EksError::ClusterNameNotFound);

        // AWS probe: a service-account mount only proves Kubernetes, which GKE,
        // AKS and self-managed clusters have too. Something has to tie the pod
        // to AWS before `cloud.platform` may claim EKS.
        let Some(cluster_name) = warn_on_error(DETECTOR, cluster_name) else {
            // Kubernetes, but nothing says EKS: return empty resource
            return Resource::builder_empty().build();
        };

        // The cluster ARN needs a partition, which costs an extra IMDS request,
        // so it is only fetched once the rest of the ARN is known.
        // If partition is not retrievable from IMDS, assume "aws".
        let cluster_arn = match (&region, &account_id) {
            (Some(region), Some(account_id)) => {
                let partition = imds
                    .as_ref()
                    .and_then(|imds| warn_on_error(DETECTOR, imds.get("services/partition")))
                    .and_then(non_empty)
                    .unwrap_or_else(|| "aws".to_owned());

                Some(format!(
                    "arn:{partition}:eks:{region}:{account_id}:cluster/{cluster_name}"
                ))
            }
            _ => None,
        };

        let attribute_options = [
            Some(KeyValue::new(semco::CLOUD_PROVIDER, "aws")),
            Some(KeyValue::new(semco::CLOUD_PLATFORM, "aws_eks")),
            // Namespace — from the Kubernetes service-account token mount
            Some(KeyValue::new(semco::K8S_NAMESPACE_NAME, namespace)),
            // Pod name — HOSTNAME is set to the pod name in standard k8s pods
            opt_kv(semco::K8S_POD_NAME, std::env::var("HOSTNAME").ok()),
            // Pod UID — requires the downward API to expose metadata.uid as POD_UID
            opt_kv(semco::K8S_POD_UID, std::env::var("POD_UID").ok()),
            // Node name — requires the downward API to expose spec.nodeName as NODE_NAME
            opt_kv(semco::K8S_NODE_NAME, std::env::var("NODE_NAME").ok()),
            Some(KeyValue::new(semco::K8S_CLUSTER_NAME, cluster_name)),
            opt_kv(semco::AWS_EKS_CLUSTER_ARN, cluster_arn),
            // Container ID — from cgroup, then from the mount table
            opt_kv(
                semco::CONTAINER_ID,
                warn_on_error(DETECTOR, get_container_id(cgroup_path, mountinfo_path)),
            ),
            opt_kv(
                semco::HOST_NAME,
                imds.as_ref()
                    .and_then(|imds| warn_on_error(DETECTOR, imds.get("hostname"))),
            ),
            opt_kv(semco::CLOUD_REGION, region),
            opt_kv(semco::CLOUD_ACCOUNT_ID, account_id),
        ];

        Resource::builder_empty()
            .with_attributes(ec2_document_attributes.into_iter().flatten())
            .with_attributes(attribute_options.into_iter().flatten())
            .build()
    }
}

/// Errors that can arise during EKS detection: filesystem reads, an empty
/// namespace file, or no container ID anywhere in the cgroup or mount files.
#[derive(Debug, Error)]
enum EksError {
    #[error("Cannot read file {path}: {error}")]
    FsError {
        path: String,
        #[source]
        error: std::io::Error,
    },
    #[error("Empty file at {K8S_NAMESPACE_FILE_PATH}")]
    EmptyNamespace,
    #[error("Could not extract the container id from {CGROUP_FILE_PATH} or {MOUNTINFO_FILE_PATH}")]
    NoContainerId,
    #[error("Could not find the cluster name, neither in the AWS EC2 tags through IMDS nor from the `{EKS_CLUSTER_NAME_ENV_VAR}` environment variable")]
    ClusterNameNotFound,
}

/// Reads and trims the service-account namespace file, erroring if the result is empty.
fn get_namespace(path: &Path) -> Result<String, EksError> {
    let namespace = std::fs::read_to_string(path)
        .map_err(|error| EksError::FsError {
            path: path.to_string_lossy().into_owned(),
            error,
        })?
        .trim()
        .to_owned();
    if namespace.is_empty() {
        Err(EksError::EmptyNamespace)
    } else {
        Ok(namespace)
    }
}

/// Reads the container ID from the cgroup file, falling back to the mount table
/// when the cgroup file carries none (cgroup v2 namespace, where the pod only sees `0::/`).
///
/// The files, extraction rules and accepted ID shape mirror `ContainerResourceDetector`
/// in `opentelemetry-resource-detectors` to ensure both detectors agree on `container.id`.
fn get_container_id(cgroup_path: &Path, mountinfo_path: &Path) -> Result<String, EksError> {
    if let Ok(content) = std::fs::read_to_string(cgroup_path) {
        if let Some(id) = content.lines().find_map(container_id_from_cgroup_line) {
            return Ok(id.to_owned());
        }
    }
    if let Ok(content) = std::fs::read_to_string(mountinfo_path) {
        if let Some(id) = content.lines().find_map(container_id_from_mountinfo_line) {
            return Ok(id.to_owned());
        }
    }
    Err(EksError::NoContainerId)
}

/// Extracts a container ID from a single cgroup line, or `None` if the line carries none.
///
/// Handles the known path layouts:
/// - `.../docker/<id>` — plain Docker, cgroup v1
/// - `.../kubepods/besteffort/pod<uid>/<id>` — Kubernetes, cgroupfs driver
/// - `.../docker-<id>.scope` — Docker, systemd driver
/// - `.../cri-containerd-<id>.scope` — containerd, systemd driver
/// - `.../crio-<id>.scope` — CRI-O, systemd driver
///
/// The runtime prefix is stripped at the last `:` or `-`, the suffix at the first `.`,
/// and the result is accepted only if it passes [`is_valid_container_id`].
fn container_id_from_cgroup_line(line: &str) -> Option<&str> {
    let last_segment = line[line.rfind('/')? + 1..].trim();

    let candidate = match last_segment.rfind([':', '-']) {
        Some(index) => &last_segment[index + 1..],
        None => last_segment,
    };

    let candidate = candidate
        .split_once('.')
        .map_or(candidate, |(before, _)| before);

    is_valid_container_id(candidate).then_some(candidate)
}

/// Extracts a container ID from a single mount table line, or `None` if the line
/// carries no container ID.
///
/// The ID is the segment following `containers` or `overlay-containers` in the
/// root of the `/etc/hostname` mount, which every container runtime bind-mounts
/// from its own per-container directory.
fn container_id_from_mountinfo_line(line: &str) -> Option<&str> {
    // Root and mount point precede the " - " separator and are indexed 3 and 4 when split by whitespace.
    let mut fields = line.split_once(" - ")?.0.split_whitespace();
    let root = fields.nth(3)?;
    let mount_point = fields.next()?;
    if mount_point != "/etc/hostname" {
        return None;
    }

    let mut previous = "";
    for segment in root.split('/') {
        if matches!(previous, "containers" | "overlay-containers") && is_valid_container_id(segment)
        {
            return Some(segment);
        }
        previous = segment;
    }

    None
}

/// Checks that a candidate is a hexadecimal string of a plausible length for a
/// container ID.
fn is_valid_container_id(candidate: &str) -> bool {
    (MIN_CONTAINER_ID_LEN..=MAX_CONTAINER_ID_LEN).contains(&candidate.len())
        && candidate.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detector::imds::{tests::FakeImdsClient, ImdsError};
    use sealed_test::prelude::*;
    use std::io::Write;

    // A 64-char all-lowercase hex string used as the canonical container ID in tests.
    const ID64: &str = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
    // A 32-char all-lowercase hex string.
    const ID32: &str = "aabbccddeeff00112233445566778899";

    // ── IMDS JSON fixture ─────────────────────────────────────────────────────

    /// Relevant instance identity document for an EKS node.
    const FULL_IMDS_DOC: &str = r#"
        {
            "accountId":        "123456789012",
            "region":           "us-east-1",
            "availabilityZone": "us-east-1c",
            "instanceId":       "i-0node",
            "instanceType":     "m5.large",
            "imageId":          "ami-0nodeimage",
            "architecture":     "x86_64"
        }
    "#;

    // ---------------------------------------------------------------------------
    // is_valid_container_id
    // ---------------------------------------------------------------------------

    #[test]
    fn is_valid_container_id_valid() {
        // 32-char all-hex string (minimum valid length)
        assert!(is_valid_container_id(ID32));

        // 64-char all-hex string (maximum valid length)
        assert!(is_valid_container_id(ID64));

        // Mixed-case hex string of valid length (46 chars)
        let mixed = "aAbBcCdDeEfF001122334455aAbBcCdDeEfF0011223344";
        assert!(is_valid_container_id(mixed));
    }

    #[test]
    fn is_valid_container_id_invalid() {
        // Too short: 31 hex chars (one below the 32-char minimum)
        let short31 = "aabbccddeeff0011223344556677889";
        assert_eq!(short31.len(), 31);
        assert!(!is_valid_container_id(short31));

        // Too long: 65 hex chars
        let long65 = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899a";
        assert_eq!(long65.len(), 65);
        assert!(!is_valid_container_id(long65));

        // Correct length (64) but contains a non-hex char 'g'
        let with_g = "aabbccddeeff00112233445566778899aabbccddeeff0011223344556677889g";
        assert_eq!(with_g.len(), 64);
        assert!(!is_valid_container_id(with_g));

        // Correct length (64) but contains a dash '-'
        let with_dash = "aabbccddeeff00112233445566778899aabbccddeeff001122334455667788-9";
        assert_eq!(with_dash.len(), 64);
        assert!(!is_valid_container_id(with_dash));

        // Empty string
        assert!(!is_valid_container_id(""));
    }

    // ---------------------------------------------------------------------------
    // container_id_from_cgroup_line
    // ---------------------------------------------------------------------------

    #[test]
    fn cgroup_line_plain_docker_v1() {
        // Plain Docker cgroup v1: `12:cpuset:/docker/<ID>`
        let line = format!("12:cpuset:/docker/{ID64}");
        assert_eq!(container_id_from_cgroup_line(&line), Some(ID64));
    }

    #[test]
    fn cgroup_line_kubernetes_cgroupfs() {
        // Kubernetes cgroupfs driver: `.../kubepods/besteffort/pod<uid>/<ID>`
        let line = format!(
            "11:memory:/kubepods/besteffort/podabcd1234-ef56-7890-abcd-ef1234567890/{ID64}"
        );
        assert_eq!(container_id_from_cgroup_line(&line), Some(ID64));
    }

    #[test]
    fn cgroup_line_docker_systemd() {
        // Docker systemd driver: `.../docker-<ID>.scope`
        let line = format!("10:cpu:/system.slice/docker-{ID64}.scope");
        assert_eq!(container_id_from_cgroup_line(&line), Some(ID64));
    }

    #[test]
    fn cgroup_line_containerd_systemd() {
        // containerd systemd driver: `.../cri-containerd-<ID>.scope`
        let line = format!("9:cpuset:/system.slice/cri-containerd-{ID64}.scope");
        assert_eq!(container_id_from_cgroup_line(&line), Some(ID64));
    }

    #[test]
    fn cgroup_line_crio_systemd() {
        // CRI-O systemd driver: `.../crio-<ID>.scope`
        let line = format!("8:memory:/system.slice/crio-{ID64}.scope");
        assert_eq!(container_id_from_cgroup_line(&line), Some(ID64));
    }

    #[test]
    fn cgroup_line_rejected() {
        // cgroup v2 root entry with no container ID
        assert_eq!(container_id_from_cgroup_line("0::/"), None);

        // Last segment is non-hex (contains letters outside a-f)
        assert_eq!(
            container_id_from_cgroup_line("1:name=systemd:/user.slice/user-1000.slice"),
            None
        );

        // Last segment is a valid-looking path but the ID contains non-hex chars
        let bad_id = "aabbccddeeff00112233445566778899aabbccddeeff001122334455667788zz";
        let line = format!("12:cpuset:/docker/{bad_id}");
        assert_eq!(container_id_from_cgroup_line(&line), None);
    }

    // ---------------------------------------------------------------------------
    // container_id_from_mountinfo_line
    // ---------------------------------------------------------------------------

    // Helper: build a mountinfo line with the given root and mount_point.
    // Format: `mountID parentID major:minor root mountPoint options - fstype source superOpts`
    // Fields before " - ": [0]=mountID [1]=parentID [2]=major:minor [3]=root [4]=mountPoint [5]=options
    fn mountinfo_line(root: &str, mount_point: &str) -> String {
        format!("36 35 0:33 {root} {mount_point} rw - ext4 /dev/sda1 rw")
    }

    #[test]
    fn mountinfo_line_containers_valid() {
        // root contains `.../containers/<ID>/...`, mount_point is /etc/hostname
        let root = format!("/docker/containers/{ID64}/hostname");
        let line = mountinfo_line(&root, "/etc/hostname");
        assert_eq!(container_id_from_mountinfo_line(&line), Some(ID64));
    }

    #[test]
    fn mountinfo_line_overlay_containers_valid() {
        // root uses `overlay-containers/<ID>/...`
        let root = format!("/var/lib/overlay-containers/{ID64}/userdata/hostname");
        let line = mountinfo_line(&root, "/etc/hostname");
        assert_eq!(container_id_from_mountinfo_line(&line), Some(ID64));
    }

    #[test]
    fn mountinfo_line_wrong_mount_point() {
        // mount_point is not /etc/hostname -> None
        let root = format!("/docker/containers/{ID64}/hostname");
        let line = mountinfo_line(&root, "/etc/hosts");
        assert_eq!(container_id_from_mountinfo_line(&line), None);
    }

    #[test]
    fn mountinfo_line_no_separator() {
        // No " - " separator in the line -> None
        let line = format!("36 35 0:33 /docker/containers/{ID64}/hostname /etc/hostname rw");
        assert_eq!(container_id_from_mountinfo_line(&line), None);
    }

    #[test]
    fn mountinfo_line_invalid_id_after_containers() {
        // "containers" is present but the following segment is not a valid container ID
        let root = "/docker/containers/not-a-valid-hex-id/hostname";
        let line = mountinfo_line(root, "/etc/hostname");
        assert_eq!(container_id_from_mountinfo_line(&line), None);
    }

    // ── detect_from helpers ───────────────────────────────────────────────────

    /// Creates a temp file with the given content and returns its path.
    fn temp_file(content: &str) -> (tempfile::NamedTempFile, std::path::PathBuf) {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "{content}").unwrap();
        let path = f.path().to_path_buf();
        (f, path)
    }

    // ── Not Kubernetes → empty resource ──────────────────────────────────────

    #[sealed_test]
    fn detect_from_no_namespace_file_returns_empty() {
        let resource = EksResourceDetector::detect_from(
            Ok(FakeImdsClient::new()),
            Path::new("/nonexistent/path/to/namespace"),
            Path::new("/nonexistent/cgroup"),
            Path::new("/nonexistent/mountinfo"),
        );
        assert_eq!(resource, Resource::builder_empty().build());
    }

    // ── Kubernetes but no AWS tie → empty resource ────────────────────────────

    #[sealed_test]
    fn detect_from_no_aws_tie_returns_empty() {
        let (_ns_file, ns_path) = temp_file("default");
        let (_cgroup_file, cgroup_path) = temp_file("0::/\n");
        let (_mi_file, mi_path) = temp_file("");

        // IMDS fails, no AWS_CLUSTER_NAME → second probe fails
        temp_env::with_vars(
            [
                ("AWS_CLUSTER_NAME", None::<&str>),
                ("AWS_REGION", None::<&str>),
                ("AWS_ACCOUNT_ID", None::<&str>),
            ],
            || {
                let resource = EksResourceDetector::detect_from(
                    Err::<FakeImdsClient, _>(ImdsError::EmptyAuthToken),
                    &ns_path,
                    &cgroup_path,
                    &mi_path,
                );
                assert_eq!(resource, Resource::builder_empty().build());
            },
        );
    }

    // ── Happy path with IMDS: full attrs ─────────────────────────────────────

    #[sealed_test]
    fn detect_from_happy_path_with_imds() {
        let (_ns_file, ns_path) = temp_file("kube-system");
        // cgroup with a valid container ID
        let (_cgroup_file, cgroup_path) =
            temp_file(&format!("11:memory:/kubepods/besteffort/podabc/{ID64}\n"));
        let (_mi_file, mi_path) = temp_file("");

        let fake = FakeImdsClient::new()
            .with_document(FULL_IMDS_DOC)
            .with_get(EKS_CLUSTER_NAME_TAG_PATH, "my-eks-cluster")
            .with_get("services/partition", "aws")
            .with_get("hostname", "ip-10-0-1-5.ec2.internal");

        temp_env::with_vars(
            [
                ("HOSTNAME", Some("my-pod-xyz")),
                ("POD_UID", Some("pod-uid-123")),
                ("NODE_NAME", Some("ip-10-0-1-5")),
                ("AWS_CLUSTER_NAME", None::<&str>),
                ("AWS_REGION", None::<&str>),
                ("AWS_ACCOUNT_ID", None::<&str>),
            ],
            || {
                let resource =
                    EksResourceDetector::detect_from(Ok(fake), &ns_path, &cgroup_path, &mi_path);

                let expected = Resource::builder_empty()
                    .with_attributes([
                        KeyValue::new(semco::CLOUD_PROVIDER, "aws"),
                        KeyValue::new(semco::CLOUD_PLATFORM, "aws_eks"),
                        KeyValue::new(semco::HOST_ARCH, "amd64"),
                        KeyValue::new(semco::CLOUD_AVAILABILITY_ZONE, "us-east-1c"),
                        KeyValue::new(semco::HOST_ID, "i-0node"),
                        KeyValue::new(semco::HOST_TYPE, "m5.large"),
                        KeyValue::new(semco::HOST_IMAGE_ID, "ami-0nodeimage"),
                        KeyValue::new(semco::K8S_NAMESPACE_NAME, "kube-system"),
                        KeyValue::new(semco::K8S_POD_NAME, "my-pod-xyz"),
                        KeyValue::new(semco::K8S_POD_UID, "pod-uid-123"),
                        KeyValue::new(semco::K8S_NODE_NAME, "ip-10-0-1-5"),
                        KeyValue::new(semco::K8S_CLUSTER_NAME, "my-eks-cluster"),
                        KeyValue::new(
                            semco::AWS_EKS_CLUSTER_ARN,
                            "arn:aws:eks:us-east-1:123456789012:cluster/my-eks-cluster",
                        ),
                        KeyValue::new(semco::CONTAINER_ID, ID64),
                        KeyValue::new(semco::HOST_NAME, "ip-10-0-1-5.ec2.internal"),
                        KeyValue::new(semco::CLOUD_REGION, "us-east-1"),
                        KeyValue::new(semco::CLOUD_ACCOUNT_ID, "123456789012"),
                    ])
                    .build();

                assert_eq!(resource, expected);
            },
        );
    }

    // ── Fargate fallback: IMDS fails, env vars supply region/account/cluster ──

    #[sealed_test]
    fn detect_from_fargate_fallback_via_env_vars() {
        let (_ns_file, ns_path) = temp_file("default");
        let (_cgroup_file, cgroup_path) = temp_file("0::/\n");
        let (_mi_file, mi_path) = temp_file("");

        temp_env::with_vars(
            [
                ("AWS_CLUSTER_NAME", Some("fargate-cluster")),
                ("AWS_REGION", Some("eu-west-1")),
                ("AWS_ACCOUNT_ID", Some("999888777666")),
                ("HOSTNAME", Some("fargate-pod-abc")),
                ("POD_UID", None::<&str>),
                ("NODE_NAME", None::<&str>),
            ],
            || {
                let resource = EksResourceDetector::detect_from(
                    Err::<FakeImdsClient, _>(ImdsError::EmptyAuthToken),
                    &ns_path,
                    &cgroup_path,
                    &mi_path,
                );

                // Partition falls back to "aws" when IMDS unavailable
                let expected = Resource::builder_empty()
                    .with_attributes([
                        KeyValue::new(semco::CLOUD_PROVIDER, "aws"),
                        KeyValue::new(semco::CLOUD_PLATFORM, "aws_eks"),
                        KeyValue::new(semco::K8S_NAMESPACE_NAME, "default"),
                        KeyValue::new(semco::K8S_POD_NAME, "fargate-pod-abc"),
                        KeyValue::new(semco::K8S_CLUSTER_NAME, "fargate-cluster"),
                        KeyValue::new(
                            semco::AWS_EKS_CLUSTER_ARN,
                            "arn:aws:eks:eu-west-1:999888777666:cluster/fargate-cluster",
                        ),
                        KeyValue::new(semco::CLOUD_REGION, "eu-west-1"),
                        KeyValue::new(semco::CLOUD_ACCOUNT_ID, "999888777666"),
                    ])
                    .build();

                assert_eq!(resource, expected);
            },
        );
    }

    // ── container.id: cgroup file first, then mountinfo ───────────────────────

    #[sealed_test]
    fn detect_from_container_id_from_mountinfo_when_cgroup_empty() {
        let (_ns_file, ns_path) = temp_file("default");
        // cgroup v2: only "0::/" — no container ID
        let (_cgroup_file, cgroup_path) = temp_file("0::/\n");
        // mountinfo with a valid container ID in /etc/hostname mount
        let root = format!("/docker/containers/{ID64}/hostname");
        let mountinfo_line_str = format!("36 35 0:33 {root} /etc/hostname rw - ext4 /dev/sda1 rw");
        let (_mi_file, mi_path) = temp_file(&mountinfo_line_str);

        let fake = FakeImdsClient::new()
            .with_document(FULL_IMDS_DOC)
            .with_get(EKS_CLUSTER_NAME_TAG_PATH, "my-cluster")
            .with_get("services/partition", "aws");

        temp_env::with_vars(
            [
                ("HOSTNAME", None::<&str>),
                ("POD_UID", None::<&str>),
                ("NODE_NAME", None::<&str>),
                ("AWS_CLUSTER_NAME", None::<&str>),
                ("AWS_REGION", None::<&str>),
                ("AWS_ACCOUNT_ID", None::<&str>),
            ],
            || {
                let resource =
                    EksResourceDetector::detect_from(Ok(fake), &ns_path, &cgroup_path, &mi_path);

                let attributes: std::collections::HashMap<_, _> = resource
                    .iter()
                    .map(|(k, v)| (k.as_str().to_owned(), v.clone()))
                    .collect();

                assert_eq!(
                    attributes.get(semco::CONTAINER_ID).map(|v| v.as_str()),
                    Some(ID64.into())
                );
            },
        );
    }

    #[sealed_test]
    fn detect_from_container_id_absent_when_no_cgroup_or_mountinfo() {
        let (_ns_file, ns_path) = temp_file("default");
        let (_cgroup_file, cgroup_path) = temp_file("0::/\n");
        let (_mi_file, mi_path) = temp_file(""); // no container ID

        let fake = FakeImdsClient::new()
            .with_document(FULL_IMDS_DOC)
            .with_get(EKS_CLUSTER_NAME_TAG_PATH, "my-cluster")
            .with_get("services/partition", "aws");

        temp_env::with_vars(
            [
                ("HOSTNAME", None::<&str>),
                ("POD_UID", None::<&str>),
                ("NODE_NAME", None::<&str>),
                ("AWS_CLUSTER_NAME", None::<&str>),
                ("AWS_REGION", None::<&str>),
                ("AWS_ACCOUNT_ID", None::<&str>),
            ],
            || {
                let resource =
                    EksResourceDetector::detect_from(Ok(fake), &ns_path, &cgroup_path, &mi_path);

                let attributes: std::collections::HashMap<_, _> = resource
                    .iter()
                    .map(|(k, v)| (k.as_str().to_owned(), v.clone()))
                    .collect();

                assert!(
                    !attributes.contains_key(semco::CONTAINER_ID),
                    "container.id should be absent when cgroup and mountinfo carry none"
                );
            },
        );
    }
}
