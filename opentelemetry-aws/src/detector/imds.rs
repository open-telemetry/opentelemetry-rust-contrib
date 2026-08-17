//! Shared IMDSv2 client for the EC2 instance metadata service.
//!
//! IMDSv2 requires a two-step flow:
//!   1. PUT /latest/api/token with X-aws-ec2-metadata-token-ttl-seconds header
//!      to obtain a session token.
//!   2. GET /latest/{path} with X-aws-ec2-metadata-token header set
//!      to the token obtained in step 1.
//!
//! This module provides a thin wrapper that handles both steps.

// `http` is re-exported by `ureq`, so that the detector features do not have to
// depend on the `http` crate, which only the `trace` feature pulls in.
use ureq::http::Response;
use ureq::{Agent as HttpClient, Body, Error as HttpClientError};

use thiserror::Error;

use super::utils::{blocking_client, non_empty};

/// Base URL for the EC2 instance metadata service.
const IMDS_BASE: &str = "http://169.254.169.254";
/// IMDSv2 token endpoint path.
const IMDS_TOKEN_PATH: &str = "latest/api/token";
/// Path of the instance identity document, relative to `/latest/`.
const IMDS_IDENTITY_DOCUMENT_PATH: &str = "dynamic/instance-identity/document";
/// Request header used to specify the token TTL when acquiring an IMDSv2 token.
const IMDS_TTL_HEADER: &str = "X-aws-ec2-metadata-token-ttl-seconds";
/// Request header used to pass the IMDSv2 session token on metadata requests.
const IMDS_TOKEN_HEADER: &str = "X-aws-ec2-metadata-token";
/// Token TTL in seconds (60s).
const IMDS_TOKEN_TTL: &str = "60";
/// HTTP request timeout in seconds for IMDS calls.
const IMDS_TIMEOUT_SECS: u64 = 1;

/// Errors that can arise interacting with the IMDSv2 service,
/// mostly for display purposes.
#[derive(Debug, Error)]
pub(super) enum ImdsError {
    #[error("Could not retrieve an IMDSv2 auth token: {0}")]
    AuthToken(#[source] HttpClientError),
    #[error("The IMDSv2 auth token endpoint answered with an empty body")]
    EmptyAuthToken,
    #[error("Could not GET {url}: {error}")]
    GetRequest {
        url: String,
        #[source]
        error: HttpClientError,
    },
    #[error("Could not read text response: {0}")]
    TextResponseRead(#[source] HttpClientError),
    #[error("Could not read JSON response: {0}")]
    JsonResponseRead(#[source] HttpClientError),
}

/// Abstraction over the IMDSv2 client, used to inject fakes in tests.
pub(super) trait ImdsProvider {
    fn get(&self, path: &str) -> Result<String, ImdsError>;
    fn get_identity_document(&self) -> Result<InstanceIdentityDocument, ImdsError>;
}

/// IMDSv2 session holding an HTTP client and an acquired session token.
pub(super) struct ImdsClient {
    client: HttpClient,
    token: String,
}

impl ImdsClient {
    /// Builds a blocking HTTP client with a 1s timeout and acquires an IMDSv2 session token.
    pub(super) fn new() -> Result<Self, ImdsError> {
        let client = blocking_client(std::time::Duration::from_secs(IMDS_TIMEOUT_SECS));

        // A non-2xx status is an error by default in `ureq`, which is what keeps
        // the other metadata services reachable at this link-local address —
        // Google Compute Engine and Azure both use it — from being mistaken for
        // IMDSv2. Rejecting an empty body closes the remaining gap.
        let token = client
            .put(format!("{IMDS_BASE}/{IMDS_TOKEN_PATH}"))
            .header(IMDS_TTL_HEADER, IMDS_TOKEN_TTL)
            .send_empty()
            .and_then(|mut r| r.body_mut().read_to_string())
            .map_err(ImdsError::AuthToken)?;
        let token = non_empty(token).ok_or(ImdsError::EmptyAuthToken)?;

        Ok(Self { client, token })
    }

    /// GETs a path under `/latest/` with the session token and returns the error-checked raw `Response`.
    fn get_response(&self, path: &str) -> Result<Response<Body>, ImdsError> {
        let url = format!("{IMDS_BASE}/latest/{path}");
        self.client
            .get(&url)
            .header(IMDS_TOKEN_HEADER, &self.token)
            .call()
            .map_err(|error| ImdsError::GetRequest { url, error })
    }

    /// GETs a path under `/latest/` and deserializes the JSON response body.
    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ImdsError> {
        self.get_response(path)?
            .body_mut()
            .read_json()
            .map_err(ImdsError::JsonResponseRead)
    }
}

impl ImdsProvider for ImdsClient {
    /// GETs a metadata path under `/latest/meta-data/` and returns the response body as a `String`.
    fn get(&self, path: &str) -> Result<String, ImdsError> {
        self.get_response(&format!("meta-data/{path}"))?
            .body_mut()
            .read_to_string()
            .map_err(ImdsError::TextResponseRead)
    }

    /// GETs `/latest/dynamic/instance-identity/document` and deserializes it.
    fn get_identity_document(&self) -> Result<InstanceIdentityDocument, ImdsError> {
        self.get_json(IMDS_IDENTITY_DOCUMENT_PATH)
    }
}

/// Deserialization target for the IMDSv2 instance identity document.
///
/// Only the fields consumed by the resource detectors are modelled; every field
/// is optional so that a partial document still yields the attributes it does
/// contain.
#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InstanceIdentityDocument {
    #[cfg(any(feature = "detector-aws-ec2", feature = "detector-aws-eks"))]
    pub account_id: Option<String>,
    #[cfg(any(feature = "detector-aws-ec2", feature = "detector-aws-eks"))]
    pub region: Option<String>,
    pub availability_zone: Option<String>,
    pub instance_id: Option<String>,
    pub instance_type: Option<String>,
    pub image_id: Option<String>,
    /// EC2 architecture name (`x86_64`, `arm64`, `i386`), which is *not* a
    /// `host.arch` semantic convention value. See [`Self::host_arch`].
    architecture: Option<String>,
}

impl InstanceIdentityDocument {
    /// Maps the EC2 `architecture` field onto the corresponding `host.arch`
    /// semantic convention value, returning `None` for unknown architectures.
    pub(super) fn host_arch(&self) -> Option<&'static str> {
        match self.architecture.as_deref() {
            Some("x86_64") => Some("amd64"),
            Some("arm64") => Some("arm64"),
            Some("i386") => Some("x86"),
            _ => None,
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::collections::HashMap;

    use super::*;

    // ── Fake IMDS client ──────────────────────────────────────────────────────

    /// Test-only fake that returns canned responses.
    ///
    /// Data is stored as JSON `&str` and deserialized on
    /// each call to exercises the `serde` impl alongside the detectors
    /// logic.
    pub struct FakeImdsClient {
        document: &'static str,
        gets: HashMap<&'static str, &'static str>,
    }

    impl FakeImdsClient {
        pub fn new() -> Self {
            Self {
                document: "",
                gets: HashMap::new(),
            }
        }

        pub fn with_document(mut self, json: &'static str) -> Self {
            self.document = json;
            self
        }

        /// Add/Overrides the value returned for a specific GET path.
        pub fn with_get(mut self, path: &'static str, value: &'static str) -> Self {
            self.gets.insert(path, value);
            self
        }
    }

    impl ImdsProvider for FakeImdsClient {
        fn get(&self, path: &str) -> Result<String, ImdsError> {
            self.gets
                .get(&path)
                .map(|&s| s.to_owned())
                .ok_or(ImdsError::GetRequest {
                    url: path.to_owned(),
                    error: HttpClientError::StatusCode(404),
                })
        }

        fn get_identity_document(&self) -> Result<InstanceIdentityDocument, ImdsError> {
            serde_json::from_str(self.document)
                .map_err(HttpClientError::Json)
                .map_err(ImdsError::TextResponseRead)
        }
    }

    // ── host_arch mapping ─────────────────────────────────────────────────────

    const DOC_X86_64: &str = r#"{ "architecture": "x86_64" }"#;
    const DOC_ARM64: &str = r#"{ "architecture": "arm64"  }"#;
    const DOC_I386: &str = r#"{ "architecture": "i386"   }"#;
    const DOC_MIPS: &str = r#"{ "architecture": "mips"   }"#;
    const DOC_EMPTY_ARCH: &str = r#"{ "architecture": ""    }"#;
    const DOC_NO_ARCH: &str = r#"{}"#;

    #[test]
    fn host_arch_known_mappings() {
        let doc: InstanceIdentityDocument = serde_json::from_str(DOC_X86_64).unwrap();
        assert_eq!(doc.host_arch(), Some("amd64"));

        let doc: InstanceIdentityDocument = serde_json::from_str(DOC_ARM64).unwrap();
        assert_eq!(doc.host_arch(), Some("arm64"));

        let doc: InstanceIdentityDocument = serde_json::from_str(DOC_I386).unwrap();
        assert_eq!(doc.host_arch(), Some("x86"));
    }

    #[test]
    fn host_arch_unknown_or_absent() {
        let doc: InstanceIdentityDocument = serde_json::from_str(DOC_NO_ARCH).unwrap();
        assert_eq!(doc.host_arch(), None);

        let doc: InstanceIdentityDocument = serde_json::from_str(DOC_MIPS).unwrap();
        assert_eq!(doc.host_arch(), None);

        let doc: InstanceIdentityDocument = serde_json::from_str(DOC_EMPTY_ARCH).unwrap();
        assert_eq!(doc.host_arch(), None);
    }
}
