//! Gets a verified `aws.eks.cluster.arn` from EKS `DescribeCluster`.

use std::path::Path;
use std::time::Duration;

use aws_sdk_eks::types::Cluster;
use base64::Engine;
use thiserror::Error;

use super::DETECTOR;
use crate::detector::utils::warn_on_error;

/// Path of the pod's service-account token.
const SERVICE_ACCOUNT_TOKEN_PATH: &str = "/var/run/secrets/kubernetes.io/serviceaccount/token";
/// How long to wait for EKS.
const DESCRIBE_CLUSTER_TIMEOUT: Duration = Duration::from_secs(5);

/// Gets the verified ARN for the cluster `name`, or `None`.
pub(super) fn verify(name: &str) -> Option<String> {
    warn_on_error(DETECTOR, verified_cluster_arn(name))
}

/// Looks up the cluster in EKS and returns its ARN if its OIDC issuer matches the token's.
fn verified_cluster_arn(name: &str) -> Result<String, ClusterArnError> {
    let issuer = token_issuer(Path::new(SERVICE_ACCOUNT_TOKEN_PATH))?;
    let region =
        issuer_region(&issuer).ok_or_else(|| ClusterArnError::NotEksIssuer(issuer.clone()))?;
    matching_arn(&issuer, describe_cluster(region, name)?)
}

/// The cluster's ARN, if its OIDC issuer is `issuer`. A cluster with the same name in another
/// account has a different issuer, so it fails.
fn matching_arn(issuer: &str, cluster: Cluster) -> Result<String, ClusterArnError> {
    let described = cluster
        .identity
        .and_then(|identity| identity.oidc)
        .and_then(|oidc| oidc.issuer);
    if described.as_deref() != Some(issuer) {
        return Err(ClusterArnError::IssuerMismatch {
            token: issuer.to_owned(),
            cluster: described.unwrap_or_default(),
        });
    }
    cluster
        .arn
        .ok_or_else(|| ClusterArnError::DescribeCluster("no cluster ARN".to_owned()))
}

/// Reads the issuer (`iss`) from the service-account token. The signature isn't checked,
/// since the token comes from the pod's own mount.
fn token_issuer(path: &Path) -> Result<String, ClusterArnError> {
    let token = std::fs::read_to_string(path).map_err(|error| {
        ClusterArnError::Token(format!("cannot read {}: {error}", path.display()))
    })?;
    let claims = token
        .split('.')
        .nth(1)
        .and_then(|payload| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(payload)
                .ok()
        })
        .and_then(|payload| serde_json::from_slice::<serde_json::Value>(&payload).ok());
    claims
        .as_ref()
        .and_then(|claims| claims["iss"].as_str())
        .map(str::to_owned)
        .ok_or_else(|| ClusterArnError::Token(format!("no issuer in {}", path.display())))
}

/// Gets the region from an EKS issuer like `https://oidc.eks.us-east-1.amazonaws.com/id/…`.
fn issuer_region(issuer: &str) -> Option<&str> {
    issuer
        .strip_prefix("https://oidc.eks.")?
        .split('.')
        .next()
        .filter(|region| !region.is_empty())
}

/// Calls EKS `DescribeCluster` with the default AWS credentials.
fn describe_cluster(region: &str, name: &str) -> Result<Cluster, ClusterArnError> {
    let region = aws_config::Region::new(region.to_owned());
    let name = name.to_owned();
    run_with_timeout(DESCRIBE_CLUSTER_TIMEOUT, async move {
        let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(region)
            .load()
            .await;
        aws_sdk_eks::Client::new(&config)
            .describe_cluster()
            .name(name)
            .send()
            .await
            .map_err(|error| {
                ClusterArnError::DescribeCluster(
                    aws_sdk_eks::error::DisplayErrorContext(error).to_string(),
                )
            })?
            .cluster
            .ok_or_else(|| ClusterArnError::DescribeCluster("no cluster".to_owned()))
    })
}

/// Runs `future` on its own thread, so it works from both sync and async code, and gives up
/// after `timeout`.
fn run_with_timeout<T, F>(timeout: Duration, future: F) -> Result<T, ClusterArnError>
where
    T: Send + 'static,
    F: std::future::Future<Output = Result<T, ClusterArnError>> + Send + 'static,
{
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| {
                ClusterArnError::DescribeCluster(format!("cannot start a runtime: {error}"))
            })?;
        let result = runtime.block_on(async {
            tokio::time::timeout(timeout, future)
                .await
                .unwrap_or_else(|_| {
                    Err(ClusterArnError::DescribeCluster(format!(
                        "no response within {timeout:?}"
                    )))
                })
        });
        // Don't wait for slow blocking work, like a DNS lookup.
        runtime.shutdown_background();
        result
    })
    .join()
    .unwrap_or_else(|_| {
        Err(ClusterArnError::DescribeCluster(
            "the call panicked".to_owned(),
        ))
    })
}

/// Why the cluster ARN couldn't be verified.
#[derive(Debug, Error)]
enum ClusterArnError {
    #[error("Invalid service-account token: {0}")]
    Token(String),
    #[error("The service-account token's issuer {0} is not an EKS issuer")]
    NotEksIssuer(String),
    #[error("EKS DescribeCluster failed: {0}")]
    DescribeCluster(String),
    #[error(
        "The cluster's OIDC issuer {cluster} is not the service-account token's issuer {token}"
    )]
    IssuerMismatch { token: String, cluster: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::serve_once;
    use aws_sdk_eks::types::{Identity, Oidc};
    use sealed_test::prelude::*;
    use std::io::Write;
    use std::time::Instant;

    const ISSUER: &str =
        "https://oidc.eks.us-east-1.amazonaws.com/id/EXAMPLED539D4633E53DE1B71EXAMPLE";
    const ARN: &str = "arn:aws:eks:us-east-1:123456789012:cluster/demo";

    /// A token file with the given claims. Only the middle part is read.
    fn token_file(claims: &str) -> tempfile::NamedTempFile {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims);
        file(&format!("eyJhbGciOiJSUzI1NiJ9.{payload}.c2lnbmF0dXJl"))
    }

    fn file(contents: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        file
    }

    fn cluster(arn: Option<&str>, issuer: &str) -> Cluster {
        let oidc = Oidc::builder().issuer(issuer).build();
        Cluster::builder()
            .set_arn(arn.map(str::to_owned))
            .identity(Identity::builder().oidc(oidc).build())
            .build()
    }

    #[test]
    fn token_issuer_reads_the_iss_claim() {
        let token = token_file(&format!(r#"{{"iss":"{ISSUER}","sub":"app"}}"#));
        assert_eq!(token_issuer(token.path()).unwrap(), ISSUER);
    }

    #[test]
    fn token_issuer_rejects_invalid_tokens() {
        let no_issuer = token_file(r#"{"sub":"app"}"#);
        assert!(token_issuer(no_issuer.path()).is_err());
        for contents in [
            "not-a-jwt",
            "header.!!!.signature",
            "header.bm90IGpzb24.signature",
        ] {
            assert!(token_issuer(file(contents).path()).is_err(), "{contents}");
        }
        assert!(token_issuer(Path::new("/nonexistent/token")).is_err());
    }

    #[test]
    fn issuer_region_reads_eks_issuers() {
        assert_eq!(issuer_region(ISSUER), Some("us-east-1"));
        assert_eq!(
            issuer_region("https://oidc.eks.cn-north-1.amazonaws.com.cn/id/ABC"),
            Some("cn-north-1")
        );
        assert_eq!(
            issuer_region("https://oidc.eks.us-iso-east-1.c2s.ic.gov/id/ABC"),
            Some("us-iso-east-1")
        );
        assert_eq!(issuer_region("https://kubernetes.default.svc"), None);
        assert_eq!(
            issuer_region("https://oidc.eks..amazonaws.com/id/ABC"),
            None
        );
    }

    #[test]
    fn matching_issuers_give_the_arn() {
        assert_eq!(
            matching_arn(ISSUER, cluster(Some(ARN), ISSUER)).unwrap(),
            ARN
        );
    }

    #[test]
    fn a_different_issuer_is_rejected() {
        let other = cluster(
            Some(ARN),
            "https://oidc.eks.us-east-1.amazonaws.com/id/OTHER",
        );
        assert!(matches!(
            matching_arn(ISSUER, other),
            Err(ClusterArnError::IssuerMismatch { .. })
        ));
    }

    #[test]
    fn a_cluster_without_an_arn_is_rejected() {
        assert!(matching_arn(ISSUER, cluster(None, ISSUER)).is_err());
    }

    #[sealed_test]
    fn describe_cluster_calls_eks() {
        let (endpoint, server) = serve_once(
            "200 OK",
            r#"{"cluster":{"arn":"arn:aws:eks:us-east-1:123456789012:cluster/demo","identity":{"oidc":{"issuer":"https://oidc.eks.us-east-1.amazonaws.com/id/EXAMPLED539D4633E53DE1B71EXAMPLE"}}}}"#,
        );
        let described = temp_env::with_vars(
            [
                ("AWS_ENDPOINT_URL_EKS", Some(endpoint.as_str())),
                ("AWS_ACCESS_KEY_ID", Some("AKIDEXAMPLE")),
                ("AWS_SECRET_ACCESS_KEY", Some("secret")),
                ("AWS_CONFIG_FILE", Some("/nonexistent")),
                ("AWS_SHARED_CREDENTIALS_FILE", Some("/nonexistent")),
                ("AWS_USE_FIPS_ENDPOINT", None),
                ("AWS_IGNORE_CONFIGURED_ENDPOINT_URLS", None),
                ("NO_PROXY", Some("127.0.0.1")),
                ("AWS_USE_DUALSTACK_ENDPOINT", None),
            ],
            || describe_cluster("us-east-1", "demo").unwrap(),
        );
        assert_eq!(server.join().unwrap(), "GET /clusters/demo HTTP/1.1");
        assert_eq!(matching_arn(ISSUER, described).unwrap(), ARN);
    }

    #[test]
    fn run_with_timeout_does_not_wait_for_blocking_work() {
        let started = Instant::now();
        let result = run_with_timeout(Duration::from_millis(100), async {
            tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_secs(3)))
                .await
                .unwrap();
            Ok(())
        });
        assert!(matches!(result, Err(ClusterArnError::DescribeCluster(_))));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn run_with_timeout_works_inside_an_async_runtime() {
        let result = run_with_timeout(Duration::from_secs(5), async { Ok(ARN) });
        assert_eq!(result.unwrap(), ARN);
    }
}
