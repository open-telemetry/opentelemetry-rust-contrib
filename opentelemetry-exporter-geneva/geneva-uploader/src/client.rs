//! High-level GenevaClient for user code. Wraps config_service and ingestion_service.

use crate::common::validate_user_agent_prefix;
use crate::config_service::client::{AuthMethod, GenevaConfigClient, GenevaConfigClientConfig};
use crate::ingestion_service::uploader::{GenevaUploader, GenevaUploaderConfig};
use crate::payload_encoder::lz4_chunked_compression::lz4_chunked_compression;
use crate::payload_encoder::otlp_encoder::OtlpEncoder;
use futures::stream::{self, StreamExt};
use opentelemetry_proto::tonic::logs::v1::ResourceLogs;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use std::sync::Arc;

/// Configuration for GenevaClient (user-facing)
#[derive(Clone, Debug)]
pub struct GenevaClientConfig {
    pub endpoint: String,
    pub environment: String,
    pub account: String,
    pub namespace: String,
    pub region: String,
    pub config_major_version: u32,
    pub auth_method: AuthMethod,
    pub tenant: String,
    pub role_name: String,
    pub role_instance: String,
    /// Maximum number of concurrent uploads. If None, defaults to number of CPU cores.
    pub max_concurrent_uploads: Option<usize>,
    /// User agent prefix for the application. Will be formatted as "<prefix> (GenevaUploader/0.1)".
    /// If None, defaults to "GenevaUploader/0.1".
    ///
    /// The prefix must contain only ASCII printable characters, be non-empty (after trimming),
    /// and not exceed 200 characters in length.
    ///
    /// Examples:
    /// - None: "GenevaUploader/0.1"
    /// - Some("MyApp/2.1.0"): "MyApp/2.1.0 (GenevaUploader/0.1)"
    /// - Some("ProductionService-1.0"): "ProductionService-1.0 (GenevaUploader/0.1)"
    pub user_agent_prefix: Option<&'static str>,
    // Add event name/version here if constant, or per-upload if you want them per call.
}

/// Builds a standardized User-Agent header for Geneva services
///
/// # Arguments
/// * `user_agent_prefix` - Optional user agent prefix from the client configuration
///
/// # Returns
/// * `Result<HeaderValue, String>` - A properly formatted User-Agent header value
///
/// # Format
/// - If prefix is None or empty: "GenevaUploader/0.1"
/// - If prefix is provided: "{prefix} (GenevaUploader/0.1)"
///
/// # Example
/// ```ignore
/// let header = build_user_agent_header(Some("MyApp/2.1.0"))?;
/// // Results in: "MyApp/2.1.0 (GenevaUploader/0.1)"
/// ```
pub fn build_user_agent_header(user_agent_prefix: Option<&str>) -> Result<HeaderValue, String> {
    let prefix = user_agent_prefix.unwrap_or("");

    // Validate the prefix if provided
    if !prefix.is_empty() {
        validate_user_agent_prefix(prefix)
            .map_err(|e| format!("Invalid user agent prefix: {e}"))?;
    }

    let user_agent = if prefix.is_empty() {
        "GenevaUploader/0.1".to_string()
    } else {
        format!("{prefix} (GenevaUploader/0.1)")
    };

    HeaderValue::from_str(&user_agent)
        .map_err(|e| format!("Failed to create User-Agent header: {e}"))
}

/// Builds a complete set of HTTP headers for Geneva services
///
/// # Arguments
/// * `user_agent_prefix` - Optional user agent prefix from the client configuration
///
/// # Returns
/// * `Result<HeaderMap, String>` - HTTP headers including User-Agent and Accept
pub fn build_geneva_headers(user_agent_prefix: Option<&str>) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();

    let user_agent = build_user_agent_header(user_agent_prefix)?;
    headers.insert(USER_AGENT, user_agent);
    headers.insert("accept", HeaderValue::from_static("application/json"));

    Ok(headers)
}

/// Main user-facing client for Geneva ingestion.
#[derive(Clone)]
pub struct GenevaClient {
    uploader: Arc<GenevaUploader>,
    encoder: OtlpEncoder,
    metadata: String,
    max_concurrent_uploads: usize,
}

impl GenevaClient {
    /// Construct a new client with minimal configuration. Fetches and caches ingestion info as needed.
    pub async fn new(cfg: GenevaClientConfig) -> Result<Self, String> {
        // Build config client config
        let config_client_config = GenevaConfigClientConfig {
            endpoint: cfg.endpoint,
            environment: cfg.environment.clone(),
            account: cfg.account,
            namespace: cfg.namespace.clone(),
            region: cfg.region,
            config_major_version: cfg.config_major_version,
            auth_method: cfg.auth_method,
            user_agent_prefix: cfg.user_agent_prefix,
        };
        let config_client = Arc::new(
            GenevaConfigClient::new(config_client_config)
                .map_err(|e| format!("GenevaConfigClient init failed: {e}"))?,
        );

        let source_identity = format!(
            "Tenant={}/Role={}/RoleInstance={}",
            cfg.tenant, cfg.role_name, cfg.role_instance
        );

        // Define config_version before using it
        let config_version = format!("Ver{}v0", cfg.config_major_version);

        // Metadata string for the blob
        let metadata = format!(
            "namespace={}/eventVersion={}/tenant={}/role={}/roleinstance={}",
            cfg.namespace, config_version, cfg.tenant, cfg.role_name, cfg.role_instance,
        );

        // Uploader config
        let uploader_config = GenevaUploaderConfig {
            namespace: cfg.namespace.clone(),
            source_identity,
            environment: cfg.environment,
            config_version: config_version.clone(),
            user_agent_prefix: cfg.user_agent_prefix,
        };

        let uploader = GenevaUploader::from_config_client(config_client, uploader_config)
            .await
            .map_err(|e| format!("GenevaUploader init failed: {e}"))?;
        let max_concurrent_uploads = cfg.max_concurrent_uploads.unwrap_or_else(|| {
            // TODO - Use a more sophisticated method to determine concurrency if needed
            // currently using number of CPU cores
            std::thread::available_parallelism()
                .map(|p| p.get())
                .unwrap_or(4)
        });
        Ok(Self {
            uploader: Arc::new(uploader),
            encoder: OtlpEncoder::new(),
            metadata,
            max_concurrent_uploads,
        })
    }

    /// Upload OTLP logs (as ResourceLogs).
    pub async fn upload_logs(&self, logs: &[ResourceLogs]) -> Result<(), String> {
        let log_iter = logs
            .iter()
            .flat_map(|resource_log| resource_log.scope_logs.iter())
            .flat_map(|scope_log| scope_log.log_records.iter());
        // TODO: Investigate using tokio::spawn_blocking for event encoding to avoid blocking
        // the async executor thread for CPU-intensive work.
        let blobs = self.encoder.encode_log_batch(log_iter, &self.metadata);

        // create an iterator that yields futures for each upload
        let upload_futures = blobs.into_iter().map(|batch| {
            async move {
                // TODO: Investigate using tokio::spawn_blocking for LZ4 compression to avoid blocking
                // the async executor thread for CPU-intensive work.
                let compressed_blob = lz4_chunked_compression(&batch.data).map_err(|e| {
                    format!("LZ4 compression failed: {e} Event: {}", batch.event_name)
                })?;
                self.uploader
                    .upload(compressed_blob, &batch.event_name, &batch.metadata)
                    .await
                    .map(|_| ())
                    .map_err(|e| format!("Geneva upload failed: {e} Event: {}", batch.event_name))
            }
        });
        // Execute uploads concurrently with configurable concurrency
        let errors: Vec<String> = stream::iter(upload_futures)
            .buffer_unordered(self.max_concurrent_uploads)
            .filter_map(|result| async move { result.err() })
            .collect()
            .await;

        // Return error if any uploads failed
        if !errors.is_empty() {
            return Err(format!("Upload failures: {}", errors.join("; ")));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::USER_AGENT;

    #[test]
    fn test_build_user_agent_header_without_prefix() {
        let header = build_user_agent_header(None).unwrap();
        assert_eq!(header.to_str().unwrap(), "GenevaUploader/0.1");
    }

    #[test]
    fn test_build_user_agent_header_with_empty_prefix() {
        let header = build_user_agent_header(Some("")).unwrap();
        assert_eq!(header.to_str().unwrap(), "GenevaUploader/0.1");
    }

    #[test]
    fn test_build_user_agent_header_with_valid_prefix() {
        let header = build_user_agent_header(Some("MyApp/2.1.0")).unwrap();
        assert_eq!(header.to_str().unwrap(), "MyApp/2.1.0 (GenevaUploader/0.1)");
    }

    #[test]
    fn test_build_user_agent_header_with_invalid_prefix() {
        let result = build_user_agent_header(Some("Invalid\nPrefix"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid user agent prefix"));
    }

    #[test]
    fn test_build_geneva_headers_complete() {
        let headers = build_geneva_headers(Some("TestApp/1.0")).unwrap();

        let user_agent = headers.get(USER_AGENT).unwrap();
        assert_eq!(
            user_agent.to_str().unwrap(),
            "TestApp/1.0 (GenevaUploader/0.1)"
        );

        let accept = headers.get("accept").unwrap();
        assert_eq!(accept.to_str().unwrap(), "application/json");
    }

    #[test]
    fn test_build_geneva_headers_without_prefix() {
        let headers = build_geneva_headers(None).unwrap();

        let user_agent = headers.get(USER_AGENT).unwrap();
        assert_eq!(user_agent.to_str().unwrap(), "GenevaUploader/0.1");

        let accept = headers.get("accept").unwrap();
        assert_eq!(accept.to_str().unwrap(), "application/json");
    }

    #[test]
    fn test_build_geneva_headers_with_invalid_prefix() {
        let result = build_geneva_headers(Some("Invalid\rPrefix"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid user agent prefix"));
    }
}
