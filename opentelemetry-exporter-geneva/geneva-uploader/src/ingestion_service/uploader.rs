use crate::config_service::client::{
    extract_endpoint_from_token, GenevaConfigClient, GenevaConfigClientError,
};
use crate::payload_encoder::central_blob::BatchMetadata;
use reqwest::{header, Client};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::error::Error as StdError;
use std::fmt::Write;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tracing::debug;
use url::form_urlencoded::byte_serialize;
use uuid::Uuid;

/// Error types for the Geneva Uploader
#[derive(Debug, Error)]
pub(crate) enum GenevaUploaderError {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("JSON error: {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("Config service error: {0}")]
    ConfigClient(String),
    #[allow(dead_code)]
    #[error("Upload failed with status {status}: {message}")]
    UploadFailed {
        status: u16,
        retry_after: Option<Duration>,
        message: String,
    },
    #[allow(dead_code)]
    #[error("Internal error: {0}")]
    InternalError(String),
}

impl From<GenevaConfigClientError> for GenevaUploaderError {
    fn from(err: GenevaConfigClientError) -> Self {
        // This preserves the original error message format from the code
        GenevaUploaderError::ConfigClient(format!("GenevaConfigClient error: {err}"))
    }
}

impl From<reqwest::Error> for GenevaUploaderError {
    fn from(err: reqwest::Error) -> Self {
        use std::fmt::Write;
        let mut msg = String::new();
        write!(&mut msg, "{err}").ok();

        if let Some(url) = err.url() {
            write!(msg, ", url: {url}").ok();
        }
        if let Some(status) = err.status() {
            write!(msg, ", status: {status}").ok();
        }

        // Print high-level error types
        if err.is_timeout() {
            write!(&mut msg, ", kind: timeout").ok();
        } else if err.is_connect() {
            write!(&mut msg, ", kind: connect").ok();
        } else if err.is_body() {
            write!(&mut msg, ", kind: body").ok();
        } else if err.is_decode() {
            write!(&mut msg, ", kind: decode").ok();
        } else if err.is_request() {
            write!(&mut msg, ", kind: request").ok();
        }

        // Traverse the whole source chain for detail
        let mut source = err.source();
        let mut idx = 0;
        let mut found_io = false;
        while let Some(s) = source {
            write!(msg, ", cause[{idx}]: {s}").ok();

            // Surface io::ErrorKind if found
            if let Some(io_err) = s.downcast_ref::<std::io::Error>() {
                write!(msg, " (io::ErrorKind::{:?})", io_err.kind()).ok();
                found_io = true;
            }
            source = s.source();
            idx += 1;
        }

        if !found_io {
            write!(&mut msg, ", (no io::Error in source chain)").ok();
        }

        GenevaUploaderError::Http(msg)
    }
}

pub(crate) type Result<T> = std::result::Result<T, GenevaUploaderError>;

/// Response from the ingestion API when submitting data
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct IngestionResponse {
    #[allow(dead_code)]
    pub(crate) ticket: String,
    #[serde(flatten)]
    #[allow(dead_code)]
    pub(crate) extra: HashMap<String, Value>,
}

/// Configuration for the Geneva Uploader
#[derive(Debug, Clone)]
pub(crate) struct GenevaUploaderConfig {
    pub namespace: String,
    pub source_identity: String,
    #[allow(dead_code)]
    pub environment: String,
    pub config_version: String,
}

/// Where the uploader gets the GIG credential per upload.
#[derive(Debug, Clone)]
pub(crate) enum IngestionSource {
    /// Fetch from the Geneva Config Service (the default cert/MSI path).
    ConfigClient(Arc<GenevaConfigClient>),
    /// Use an agent-fed credential; no GCS handshake.
    AgentFed(Arc<dyn crate::client::AgentFedCredentialSource>),
}

/// Credential + routing resolved for a single upload, independent of source.
struct ResolvedIngestion {
    token: String,
    gig_endpoint: String,
    moniker: String,
    endpoint_query_param: String,
}

impl IngestionSource {
    /// Resolve the per-upload credential and routing. Centralizes the
    /// source-specific branching (and the agent-fed endpoint-claim fallback)
    /// here so `upload` sees a single uniform shape and never branches.
    async fn resolve(&self) -> Result<ResolvedIngestion> {
        match self {
            IngestionSource::ConfigClient(config_client) => {
                let (auth_info, moniker_info, endpoint_query_param) =
                    config_client.get_ingestion_info().await?;
                Ok(ResolvedIngestion {
                    token: auth_info.auth_token,
                    gig_endpoint: auth_info.endpoint,
                    moniker: moniker_info.name,
                    endpoint_query_param,
                })
            }
            IngestionSource::AgentFed(source) => {
                let cred = source.current().await.ok_or_else(|| {
                    GenevaUploaderError::ConfigClient(
                        "agent-fed credential not yet provisioned by host".to_string(),
                    )
                })?;
                // The GIG ingestion gateway rejects the upload (HTTP 403,
                // "Token must have 'Endpoint' claim set to ...") unless the
                // request's `endpoint` query parameter matches the `Endpoint`
                // claim embedded in the auth token. The native GCS config-service
                // path derives that value from the token itself (see
                // get_ingestion_info -> extract_endpoint_from_token), so the
                // agent-fed path must do the same. Some valid tokens legitimately
                // omit the claim; mirror the GCS path and fall back to the
                // credential's own endpoint rather than rejecting the upload.
                let endpoint_query_param = extract_endpoint_from_token(&cred.token)
                    .unwrap_or_else(|_| cred.endpoint.clone());
                Ok(ResolvedIngestion {
                    token: cred.token,
                    gig_endpoint: cred.endpoint,
                    moniker: cred.moniker,
                    endpoint_query_param,
                })
            }
        }
    }
}

/// Client for uploading data to Geneva Ingestion Gateway (GIG)
#[derive(Debug, Clone)]
pub(crate) struct GenevaUploader {
    pub(crate) source: IngestionSource,
    pub(crate) config: GenevaUploaderConfig,
    pub(crate) http_client: Client,
}

impl GenevaUploader {
    /// Constructs a GenevaUploader by calling the GenevaConfigClient
    ///
    /// # Arguments
    /// * `config_client` - Initialized GenevaConfigClient
    /// * `uploader_config` - Static config (namespace, event, version, etc.)
    ///
    /// # Returns
    /// * `Result<GenevaUploader>` with authenticated client and resolved moniker/endpoint
    #[allow(dead_code)]
    pub(crate) fn from_config_client(
        config_client: Arc<GenevaConfigClient>,
        uploader_config: GenevaUploaderConfig,
    ) -> Result<Self> {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            header::HeaderValue::from_static("application/json"),
        );
        let client = Self::build_h1_client(headers)?;

        Ok(Self {
            source: IngestionSource::ConfigClient(config_client),
            config: uploader_config,
            http_client: client,
        })
    }

    /// Constructs a GenevaUploader from an agent-fed credential source.
    /// No `GenevaConfigClient` is created; `source` is queried per upload.
    pub(crate) fn from_agent_fed(
        source: Arc<dyn crate::client::AgentFedCredentialSource>,
        uploader_config: GenevaUploaderConfig,
    ) -> Result<Self> {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            header::HeaderValue::from_static("application/json"),
        );
        let client = Self::build_h1_client(headers)?;

        Ok(Self {
            source: IngestionSource::AgentFed(source),
            config: uploader_config,
            http_client: client,
        })
    }

    fn build_h1_client(headers: header::HeaderMap) -> Result<Client> {
        Ok(Client::builder()
            .timeout(Duration::from_secs(30))
            .default_headers(headers)
            .http1_only()
            .tcp_keepalive(Some(Duration::from_secs(60)))
            .build()?)
    }

    /// Creates the GIG upload URI with required parameters
    #[allow(dead_code)]
    #[allow(clippy::too_many_arguments)]
    fn create_upload_uri(
        &self,
        endpoint_query_param: &str,
        moniker: &str,
        data_size: usize,
        event_name: &str,
        metadata: &BatchMetadata,
        row_count: usize,
        obo_config: Option<&crate::payload_encoder::otlp_encoder::OboEventConfig>,
    ) -> Result<String> {
        // Get already formatted schema IDs and format timestamps using BatchMetadata methods
        let schema_ids = &metadata.schema_ids;
        let start_time_str = metadata.format_start_timestamp();
        let end_time_str = metadata.format_end_timestamp();

        // URL encode parameters
        // TODO - Maintain this as url-encoded in config service to avoid conversion here
        let encoded_endpoint_query_param: String =
            byte_serialize(endpoint_query_param.as_bytes()).collect();
        let encoded_source_identity: String =
            byte_serialize(self.config.source_identity.as_bytes()).collect();

        // Create a source unique ID - using a UUID to ensure uniqueness
        let source_unique_id = Uuid::new_v4();

        // Create the query string
        let mut query = String::with_capacity(512); // Preallocate enough space for the query string (decided based on expected size)
        write!(&mut query, "api/v1/ingestion/ingest?endpoint={}&moniker={}&namespace={}&event={}&version={}&sourceUniqueId={}&sourceIdentity={}&startTime={}&endTime={}&format=centralbond/lz4hc&dataSize={}&minLevel={}&schemaIds={}&rowCount={}",
            encoded_endpoint_query_param,
            moniker,
            self.config.namespace,
            event_name,
            self.config.config_version,
            source_unique_id,
            encoded_source_identity,
            start_time_str,
            end_time_str,
            data_size,
            2,
            schema_ids,
            row_count
        ).map_err(|e| GenevaUploaderError::InternalError(format!("Failed to write query string: {e}")))?;

        // Append OBO query parameters if this event has OBO config
        if let Some(config) = obo_config.filter(|c| c.is_active()) {
            write!(&mut query, "&onbehalfid={}", config.identity.trim()).map_err(|e| {
                GenevaUploaderError::InternalError(format!("Failed to write OBO identity: {e}"))
            })?;
            if let Some(ann) = config.active_annotations() {
                let encoded_annotations: String = byte_serialize(ann.as_bytes()).collect();
                write!(&mut query, "&onbehalfannotations={}", encoded_annotations).map_err(
                    |e| {
                        GenevaUploaderError::InternalError(format!(
                            "Failed to write OBO annotations: {e}"
                        ))
                    },
                )?;
            }
        }

        Ok(query)
    }

    /// Uploads data to the ingestion gateway
    ///
    /// # Arguments
    /// * `data` - The encoded data to upload (already in the required format)
    /// * `event_name` - Name of the event
    /// * `event_version` - Version of the event
    /// * `metadata` - Batch metadata containing timestamps and schema information
    /// * `row_count` - Number of rows/events in the batch
    ///
    /// # Returns
    /// * `Result<IngestionResponse>` - The response containing the ticket ID or an error
    #[allow(dead_code)]
    pub(crate) async fn upload(
        &self,
        data: Vec<u8>,
        event_name: &str,
        metadata: &BatchMetadata,
        row_count: usize,
        obo_config: Option<&crate::payload_encoder::otlp_encoder::OboEventConfig>,
    ) -> Result<IngestionResponse> {
        debug!(
            name: "uploader.upload",
            target: "geneva-uploader",
            event_name = %event_name,
            size = data.len(),
            "Starting upload"
        );

        // `endpoint_query_param` becomes the `endpoint=` query param (see
        // `create_upload_uri`). Both credential sources resolve to the same
        // shape via `IngestionSource::resolve`, so this path stays branch-free.
        let ResolvedIngestion {
            token: auth_token,
            gig_endpoint,
            moniker,
            endpoint_query_param,
        } = self.source.resolve().await?;
        let data_size = data.len();
        let upload_uri = self.create_upload_uri(
            &endpoint_query_param,
            &moniker,
            data_size,
            event_name,
            metadata,
            row_count,
            obo_config,
        )?;
        let full_url = format!("{}/{}", gig_endpoint.trim_end_matches('/'), upload_uri);

        debug!(
            name: "uploader.upload.post",
            target: "geneva-uploader",
            event_name = %event_name,
            moniker = %moniker,
            "Posting to ingestion gateway"
        );

        let response = self
            .http_client
            .post(&full_url)
            .header(header::AUTHORIZATION, format!("Bearer {auth_token}"))
            .body(data)
            .send()
            .await?;
        let status = response.status();

        // TODO: Only the delay-seconds form of Retry-After is parsed here.
        // The HTTP-date form (e.g., "Fri, 31 Dec 2027 23:59:59 GMT") is
        // silently ignored and results in None. Add support if the ingestion
        // backend ever uses that form.
        let retry_after = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs);
        let body = response.text().await?;

        if status == reqwest::StatusCode::ACCEPTED {
            let ingest_response: IngestionResponse = serde_json::from_str(&body).map_err(|e| {
                debug!(
                    name: "uploader.upload.parse_error",
                    target: "geneva-uploader",
                    error = %e,
                    "Failed to parse ingestion response"
                );
                GenevaUploaderError::SerdeJson(e)
            })?;

            debug!(
                name: "uploader.upload.success",
                target: "geneva-uploader",
                event_name = %event_name,
                ticket = %ingest_response.ticket,
                "Upload successful"
            );

            return Ok(ingest_response);
        }

        tracing::warn!(
            name: "uploader.upload.failed",
            target: "geneva-uploader",
            event_name = %event_name,
            status = status.as_u16(),
            moniker = %moniker,
            body = %body,
            "Upload failed"
        );
        Err(GenevaUploaderError::UploadFailed {
            status: status.as_u16(),
            retry_after,
            message: body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload_encoder::otlp_encoder::OboEventConfig;

    fn make_uploader() -> GenevaUploader {
        let uploader_config = GenevaUploaderConfig {
            namespace: "TestNamespace".to_string(),
            source_identity: "Tenant=Test/Role=TestRole/RoleInstance=TestInstance".to_string(),
            environment: "TestEnv".to_string(),
            config_version: "Ver1v0".to_string(),
        };
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            header::HeaderValue::from_static("application/json"),
        );
        let http_client =
            GenevaUploader::build_h1_client(headers).expect("HTTP client should build");
        use crate::config_service::client::{AuthMethod, GenevaConfigClientConfig};
        let config_client_config = GenevaConfigClientConfig {
            endpoint: "https://test.endpoint.com".to_string(),
            environment: "TestEnv".to_string(),
            account: "TestAccount".to_string(),
            namespace: "TestNamespace".to_string(),
            region: "westus2".to_string(),
            config_major_version: 1,
            auth_method: AuthMethod::SystemManagedIdentity,
            msi_resource: Some("https://monitor.azure.com".to_string()),
            test_root_ca_pem: None,
        };
        let config_client = Arc::new(
            crate::config_service::client::GenevaConfigClient::new(config_client_config)
                .expect("Config client should init"),
        );
        GenevaUploader {
            source: IngestionSource::ConfigClient(config_client),
            config: uploader_config,
            http_client,
        }
    }

    fn make_test_metadata() -> BatchMetadata {
        BatchMetadata {
            start_time: 1_700_000_000_000_000_000,
            end_time: 1_700_000_001_000_000_000,
            schema_ids: "abc123".to_string(),
        }
    }

    #[test]
    fn test_upload_uri_with_obo_identity() {
        let uploader = make_uploader();
        let metadata = make_test_metadata();
        let obo_config = OboEventConfig {
            identity: "Microsoft.TestService".to_string(),
            annotations: None,
        };
        let uri = uploader
            .create_upload_uri(
                "https://monitor.endpoint",
                "testmoniker",
                1024,
                "TestEvent",
                &metadata,
                10,
                Some(&obo_config),
            )
            .expect("URI creation should succeed");
        assert!(
            uri.contains("&onbehalfid=Microsoft.TestService"),
            "URI should contain onbehalfid, got: {}",
            uri
        );
        assert!(
            !uri.contains("&onbehalfannotations="),
            "URI should NOT contain onbehalfannotations when not set"
        );
    }

    #[test]
    fn test_upload_uri_with_obo_annotations() {
        let uploader = make_uploader();
        let metadata = make_test_metadata();
        let obo_config = OboEventConfig {
            identity: "Microsoft.TestService".to_string(),
            annotations: Some(
                r#"<Config onBehalfFields="resourceId" priority="Normal"/>"#.to_string(),
            ),
        };
        let uri = uploader
            .create_upload_uri(
                "https://monitor.endpoint",
                "testmoniker",
                1024,
                "TestEvent",
                &metadata,
                10,
                Some(&obo_config),
            )
            .expect("URI creation should succeed");
        assert!(
            uri.contains("&onbehalfid=Microsoft.TestService"),
            "URI should contain onbehalfid"
        );
        assert!(
            uri.contains("&onbehalfannotations="),
            "URI should contain onbehalfannotations, got: {}",
            uri
        );
        assert!(
            !uri.contains("<Config"),
            "Annotations should be URL-encoded"
        );
    }

    #[test]
    fn test_upload_uri_without_obo() {
        let uploader = make_uploader();
        let metadata = make_test_metadata();
        let uri = uploader
            .create_upload_uri(
                "https://monitor.endpoint",
                "testmoniker",
                1024,
                "TestEvent",
                &metadata,
                10,
                None,
            )
            .expect("URI creation should succeed");
        assert!(
            !uri.contains("onbehalfid"),
            "URI should NOT contain onbehalfid, got: {}",
            uri
        );
        assert!(
            !uri.contains("onbehalfannotations"),
            "URI should NOT contain onbehalfannotations"
        );
    }

    // ── Agent-fed upload path ────────────────────────────────────────────
    // These exercise the full uploader wire path against a local mock GIG: the
    // agent-fed token must land in the `Authorization: Bearer` header, the GCS
    // config-service handshake must be skipped, and token rotation must be
    // observed per upload.

    #[derive(Debug)]
    struct TestAgentFedSource {
        token: std::sync::Mutex<String>,
        endpoint: String,
        moniker: String,
    }

    impl TestAgentFedSource {
        fn new(token: &str, endpoint: &str, moniker: &str) -> Self {
            Self {
                token: std::sync::Mutex::new(token.to_string()),
                endpoint: endpoint.to_string(),
                moniker: moniker.to_string(),
            }
        }
        fn set_token(&self, token: &str) {
            *self.token.lock().unwrap() = token.to_string();
        }
    }

    impl crate::client::AgentFedCredentialSource for TestAgentFedSource {
        fn current(&self) -> crate::client::AgentFedCredentialFuture<'_> {
            let cred = crate::client::AgentFedCredential {
                token: self.token.lock().unwrap().clone(),
                endpoint: self.endpoint.clone(),
                moniker: self.moniker.clone(),
            };
            Box::pin(async move { Some(cred) })
        }
    }

    #[derive(Debug)]
    struct EmptyAgentFedSource;
    impl crate::client::AgentFedCredentialSource for EmptyAgentFedSource {
        fn current(&self) -> crate::client::AgentFedCredentialFuture<'_> {
            Box::pin(async { None })
        }
    }

    fn agent_fed_uploader(
        source: Arc<dyn crate::client::AgentFedCredentialSource>,
    ) -> GenevaUploader {
        let uploader_config = GenevaUploaderConfig {
            namespace: "TestNamespace".to_string(),
            source_identity: "Tenant=Test/Role=R/RoleInstance=I".to_string(),
            environment: "TestEnv".to_string(),
            config_version: "Ver2v0".to_string(),
        };
        GenevaUploader::from_agent_fed(source, uploader_config).expect("agent-fed uploader builds")
    }

    /// Build a minimal JWT (`header.payload.signature`) whose payload carries
    /// the given `Endpoint` claim. Only the payload is meaningful to
    /// `extract_endpoint_from_token`; the header and signature are placeholders.
    fn agent_fed_token_with_endpoint(endpoint: &str) -> String {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        let payload = URL_SAFE_NO_PAD.encode(format!(r#"{{"Endpoint":"{endpoint}"}}"#));
        format!("hdr.{payload}.sig")
    }

    #[tokio::test]
    async fn agent_fed_upload_uses_host_token_and_skips_gcs() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        // A claimless host token is valid: with no `Endpoint` claim to extract,
        // the upload falls back to the credential's endpoint for the `endpoint=`
        // query param (mirroring the GCS path), so a plain token still uploads.
        let token = "host-token-AAA";
        // The GCS config-service is intentionally NOT mocked. If the agent-fed
        // path attempted the handshake it would fail; a 202 here proves it was
        // skipped and the host-supplied token was used directly.
        Mock::given(method("POST"))
            .and(path("/api/v1/ingestion/ingest"))
            .and(header("authorization", format!("Bearer {token}")))
            .respond_with(ResponseTemplate::new(202).set_body_string(r#"{"ticket":"t-1"}"#))
            .expect(1)
            .mount(&mock_server)
            .await;

        let source = Arc::new(TestAgentFedSource::new(
            token,
            &mock_server.uri(),
            "test-moniker",
        ));
        let uploader = agent_fed_uploader(source);
        let metadata = make_test_metadata();

        let resp = uploader
            .upload(vec![1, 2, 3], "Log", &metadata, 1, None)
            .await;
        assert!(resp.is_ok(), "agent-fed upload should succeed: {resp:?}");
        // mock_server drop verifies exactly one POST carrying the host token.
    }

    #[tokio::test]
    async fn agent_fed_upload_reflects_token_rotation() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let token_a = "tok-A";
        let token_b = "tok-B";
        Mock::given(method("POST"))
            .and(path("/api/v1/ingestion/ingest"))
            .and(header("authorization", format!("Bearer {token_a}")))
            .respond_with(ResponseTemplate::new(202).set_body_string(r#"{"ticket":"a"}"#))
            .expect(1)
            .mount(&mock_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/ingestion/ingest"))
            .and(header("authorization", format!("Bearer {token_b}")))
            .respond_with(ResponseTemplate::new(202).set_body_string(r#"{"ticket":"b"}"#))
            .expect(1)
            .mount(&mock_server)
            .await;

        let source = Arc::new(TestAgentFedSource::new(token_a, &mock_server.uri(), "m"));
        let uploader = agent_fed_uploader(source.clone());
        let metadata = make_test_metadata();

        uploader
            .upload(vec![1], "Log", &metadata, 1, None)
            .await
            .expect("upload A");
        // Host rotates the credential; the next upload must use the new token.
        source.set_token(token_b);
        uploader
            .upload(vec![2], "Log", &metadata, 1, None)
            .await
            .expect("upload B");
        // Both `.expect(1)` mocks verify each token was used exactly once.
    }

    #[tokio::test]
    async fn agent_fed_upload_errors_when_not_provisioned() {
        let uploader = agent_fed_uploader(Arc::new(EmptyAgentFedSource));
        let metadata = make_test_metadata();
        let resp = uploader.upload(vec![1], "Log", &metadata, 1, None).await;
        assert!(
            resp.is_err(),
            "upload must error when the host has not provisioned a credential"
        );
    }

    #[tokio::test]
    async fn agent_fed_upload_endpoint_query_uses_token_claim() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        // The token carries an `Endpoint` claim that differs from the
        // credential endpoint; the `endpoint=` query param must be derived from
        // that claim, overriding the credential endpoint. The mock only matches
        // (and returns 202) when `endpoint=` equals the claim, so a successful
        // upload proves the routing.
        let token = agent_fed_token_with_endpoint("https://claim.endpoint.example");
        Mock::given(method("POST"))
            .and(path("/api/v1/ingestion/ingest"))
            .and(query_param("endpoint", "https://claim.endpoint.example"))
            .respond_with(ResponseTemplate::new(202).set_body_string(r#"{"ticket":"t"}"#))
            .expect(1)
            .mount(&mock_server)
            .await;

        let source = Arc::new(TestAgentFedSource::new(&token, &mock_server.uri(), "m"));
        let uploader = agent_fed_uploader(source);
        let metadata = make_test_metadata();
        let resp = uploader
            .upload(vec![1, 2, 3], "Log", &metadata, 1, None)
            .await;
        assert!(
            resp.is_ok(),
            "`endpoint=` must come from the token's Endpoint claim: {resp:?}"
        );
    }

    #[tokio::test]
    async fn agent_fed_upload_endpoint_query_falls_back_to_cred_endpoint() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        // A claimless token has no `Endpoint` claim, so the `endpoint=` query
        // param must fall back to the credential's own endpoint. The mock only
        // matches when `endpoint=` equals that endpoint.
        let endpoint = mock_server.uri();
        Mock::given(method("POST"))
            .and(path("/api/v1/ingestion/ingest"))
            .and(query_param("endpoint", endpoint.as_str()))
            .respond_with(ResponseTemplate::new(202).set_body_string(r#"{"ticket":"t"}"#))
            .expect(1)
            .mount(&mock_server)
            .await;

        let source = Arc::new(TestAgentFedSource::new("claimless-token", &endpoint, "m"));
        let uploader = agent_fed_uploader(source);
        let metadata = make_test_metadata();
        let resp = uploader
            .upload(vec![1, 2, 3], "Log", &metadata, 1, None)
            .await;
        assert!(
            resp.is_ok(),
            "`endpoint=` must fall back to the credential endpoint: {resp:?}"
        );
    }
}
