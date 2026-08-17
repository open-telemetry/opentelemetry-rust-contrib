use opentelemetry::KeyValue;
use opentelemetry_sdk::{resource::ResourceDetector, Resource};
use opentelemetry_semantic_conventions::attribute as semco;

use super::{
    imds::{ImdsClient, ImdsError, ImdsProvider},
    utils::{opt_kv, warn_on_error},
};

/// Name reported in internal logs emitted by this detector.
const DETECTOR: &str = "aws_ec2";

/// EC2 resource detector (`detector-aws-ec2` feature).
///
/// Queries the EC2 Instance Metadata Service v2 (IMDSv2) at the link-local
/// address `169.254.169.254` and returns an OTel [`Resource`] with the
/// following attributes:
///
/// | OTel attribute            | Source                                                         |
/// |---------------------------|----------------------------------------------------------------|
/// | `cloud.provider`          | hardcoded `"aws"`                                              |
/// | `cloud.platform`          | hardcoded `"aws_ec2"`                                          |
/// | `cloud.account.id`        | identity document `accountId`                                  |
/// | `cloud.region`            | identity document `region`                                     |
/// | `cloud.availability_zone` | identity document `availabilityZone`                           |
/// | `host.id`                 | identity document `instanceId`                                 |
/// | `host.type`               | identity document `instanceType`                               |
/// | `host.image.id`           | identity document `imageId`                                    |
/// | `host.arch`               | identity document `architecture`, mapped to `host.arch` values |
/// | `host.name`               | `/latest/meta-data/hostname`                                   |
///
/// All but `host.name` come from the instance identity document
/// (`/latest/dynamic/instance-identity/document`), so detection costs three
/// requests in total: the IMDSv2 token, the document and the hostname.
///
/// Values that cannot be found are skipped.
///
/// # Feature flag
///
/// This type is only available when the `detector-aws-ec2` Cargo feature is
/// enabled.
///
/// # Behavior
///
/// Detection is best-effort. Any IMDSv2 call that fails (network error, HTTP
/// error, or JSON parse error) is silently skipped and the corresponding
/// attribute is omitted from the returned [`Resource`].
///
/// The platform probe is the acquisition of an IMDSv2 session token *and* the
/// successful parse of the instance identity document. Unless both succeed the
/// environment is assumed not to be EC2 and an empty [`Resource`] is returned,
/// so that `cloud.provider` and `cloud.platform` are never asserted
/// off-platform.
///
/// # Blocking
///
/// Detection performs blocking HTTP requests, each capped by a one second
/// timeout, and will therefore stall the calling thread.
///
/// # Examples
///
/// Register the detector with the OpenTelemetry SDK so that EC2 attributes are
/// automatically merged into the global resource:
///
/// ```no_run
/// // Requires a running EC2 instance with IMDSv2 enabled.
/// use opentelemetry_aws::detector::Ec2ResourceDetector;
/// use opentelemetry_sdk::Resource;
///
/// let resource = Resource::builder()
///     .with_detector(Box::new(Ec2ResourceDetector))
///     .build();
/// ```
///
/// [`Resource`]: opentelemetry_sdk::Resource
pub struct Ec2ResourceDetector;

impl ResourceDetector for Ec2ResourceDetector {
    fn detect(&self) -> Resource {
        Self::detect_from(ImdsClient::new())
    }
}

impl Ec2ResourceDetector {
    fn detect_from<P: ImdsProvider>(imds: Result<P, ImdsError>) -> Resource {
        // Platform probe. A session token only proves that *something* answers
        // at the link-local address; a parsable identity document is what
        // proves it is IMDSv2. Both failures are routine off-platform.
        let Some(imds) = warn_on_error(DETECTOR, imds) else {
            // Not EC2, return empty resource
            return Resource::builder_empty().build();
        };
        let Some(document) = warn_on_error(DETECTOR, imds.get_identity_document()) else {
            // Not EC2, return empty resource
            return Resource::builder_empty().build();
        };

        // Past this point the environment is believed to be EC2.
        let attribute_options = [
            Some(KeyValue::new(semco::CLOUD_PROVIDER, "aws")),
            Some(KeyValue::new(semco::CLOUD_PLATFORM, "aws_ec2")),
            document
                .host_arch()
                .map(|arch| KeyValue::new(semco::HOST_ARCH, arch)),
            opt_kv(semco::CLOUD_ACCOUNT_ID, document.account_id),
            opt_kv(semco::CLOUD_REGION, document.region),
            opt_kv(semco::CLOUD_AVAILABILITY_ZONE, document.availability_zone),
            opt_kv(semco::HOST_ID, document.instance_id),
            opt_kv(semco::HOST_TYPE, document.instance_type),
            opt_kv(semco::HOST_IMAGE_ID, document.image_id),
            // Host name — the only attribute absent from the identity document
            opt_kv(
                semco::HOST_NAME,
                warn_on_error(DETECTOR, imds.get("hostname")),
            ),
        ];

        Resource::builder_empty()
            .with_attributes(attribute_options.into_iter().flatten())
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detector::imds::tests::FakeImdsClient;

    // ── IMDS JSON fixtures ────────────────────────────────────────────────────

    /// Identity document with all relevant fields.
    const FULL_DOC: &str = r#"
        {
            "accountId":        "123456789012",
            "region":           "us-east-1",
            "availabilityZone": "us-east-1a",
            "instanceId":       "i-0abcdef1234567890",
            "instanceType":     "m5.xlarge",
            "imageId":          "ami-0abcdef1234567890",
            "architecture":     "x86_64"
        }
    "#;

    /// Sparse document — only instanceId and arm64 architecture; all other
    /// fields absent, so only host.id, host.arch and host.name should appear.
    const SPARSE_DOC_ARM64: &str = r#"
        {
            "instanceId":   "i-0abcdef1234567890",
            "architecture": "arm64"
        }
    "#;

    /// Document with an unknown architecture — host.arch must be omitted.
    const DOC_UNKNOWN_ARCH: &str = r#"
        {
            "accountId":    "123456789012",
            "region":       "us-west-2",
            "instanceId":   "i-0aaa",
            "architecture": "mips"
        }
    "#;

    // ── Happy path ────────────────────────────────────────────────────────────

    #[test]
    fn detect_from_full_document_and_hostname() {
        let fake = FakeImdsClient::new()
            .with_document(FULL_DOC)
            .with_get("hostname", "ip-10-0-1-2.ec2.internal");

        let resource = Ec2ResourceDetector::detect_from(Ok(fake));

        let expected = Resource::builder_empty()
            .with_attributes([
                KeyValue::new(semco::CLOUD_PROVIDER, "aws"),
                KeyValue::new(semco::CLOUD_PLATFORM, "aws_ec2"),
                KeyValue::new(semco::HOST_ARCH, "amd64"),
                KeyValue::new(semco::CLOUD_ACCOUNT_ID, "123456789012"),
                KeyValue::new(semco::CLOUD_REGION, "us-east-1"),
                KeyValue::new(semco::CLOUD_AVAILABILITY_ZONE, "us-east-1a"),
                KeyValue::new(semco::HOST_ID, "i-0abcdef1234567890"),
                KeyValue::new(semco::HOST_TYPE, "m5.xlarge"),
                KeyValue::new(semco::HOST_IMAGE_ID, "ami-0abcdef1234567890"),
                KeyValue::new(semco::HOST_NAME, "ip-10-0-1-2.ec2.internal"),
            ])
            .build();

        assert_eq!(resource, expected);
    }

    // ── Construction failure → empty resource ─────────────────────────────────

    #[test]
    fn detect_from_construction_failure_returns_empty() {
        let resource =
            Ec2ResourceDetector::detect_from::<FakeImdsClient>(Err(ImdsError::EmptyAuthToken));

        assert_eq!(resource, Resource::builder_empty().build());
    }

    // ── Identity document error → empty resource ──────────────────────────────

    #[test]
    fn detect_from_document_error_returns_empty() {
        let fake = FakeImdsClient::new();
        let resource = Ec2ResourceDetector::detect_from(Ok(fake));

        assert_eq!(resource, Resource::builder_empty().build());
    }

    // ── Partial document: missing fields omitted ──────────────────────────────

    #[test]
    fn detect_from_partial_document_omits_missing_fields() {
        let fake = FakeImdsClient::new()
            .with_document(SPARSE_DOC_ARM64)
            .with_get("hostname", "ip-10-0-0-1.ec2.internal");

        let resource = Ec2ResourceDetector::detect_from(Ok(fake));

        let expected = Resource::builder_empty()
            .with_attributes([
                KeyValue::new(semco::CLOUD_PROVIDER, "aws"),
                KeyValue::new(semco::CLOUD_PLATFORM, "aws_ec2"),
                KeyValue::new(semco::HOST_ARCH, "arm64"),
                KeyValue::new(semco::HOST_ID, "i-0abcdef1234567890"),
                KeyValue::new(semco::HOST_NAME, "ip-10-0-0-1.ec2.internal"),
            ])
            .build();

        assert_eq!(resource, expected);
    }

    // ── Unknown arch omitted, hostname GET error omitted ──────────────────────

    #[test]
    fn detect_from_unknown_arch_omitted() {
        // GET IMDS/hostname will return a 404 NotFound
        let fake = FakeImdsClient::new().with_document(DOC_UNKNOWN_ARCH);

        let resource = Ec2ResourceDetector::detect_from(Ok(fake));

        let expected = Resource::builder_empty()
            .with_attributes([
                KeyValue::new(semco::CLOUD_PROVIDER, "aws"),
                KeyValue::new(semco::CLOUD_PLATFORM, "aws_ec2"),
                KeyValue::new(semco::CLOUD_ACCOUNT_ID, "123456789012"),
                KeyValue::new(semco::CLOUD_REGION, "us-west-2"),
                KeyValue::new(semco::HOST_ID, "i-0aaa"),
                // No HOST_NAME
                // No HOST_ARCH
            ])
            .build();

        assert_eq!(resource, expected);
    }
}
