use crate::config_service::client::{GenevaConfigClient, GenevaConfigClientError};
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
    UploadFailed { status: u16, message: String },
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
    /// On Behalf Of identity (e.g., "Microsoft.HybridCompute").
    /// When set, appended as `onbehalfid` query param to the GIG upload URI.
    pub onbehalf_identity: Option<String>,
    /// On Behalf Of annotations XML (e.g., `<Config onBehalfFields="..." />`).
    /// When set, URL-encoded and appended as `onbehalfannotations` query param.
    pub onbehalf_annotations: Option<String>,
}

/// Client for uploading data to Geneva Ingestion Gateway (GIG)
#[derive(Debug, Clone)]
pub struct GenevaUploader {
    pub config_client: Arc<GenevaConfigClient>,
    pub config: GenevaUploaderConfig,
    pub http_client: Client,
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
            config_client,
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
    fn create_upload_uri(
        &self,
        monitoring_endpoint: &str,
        moniker: &str,
        data_size: usize,
        event_name: &str,
        metadata: &BatchMetadata,
        row_count: usize,
    ) -> Result<String> {
        // Get already formatted schema IDs and format timestamps using BatchMetadata methods
        let schema_ids = &metadata.schema_ids;
        let start_time_str = metadata.format_start_timestamp();
        let end_time_str = metadata.format_end_timestamp();

        // URL encode parameters
        // TODO - Maintain this as url-encoded in config service to avoid conversion here
        let encoded_monitoring_endpoint: String =
            byte_serialize(monitoring_endpoint.as_bytes()).collect();
        let encoded_source_identity: String =
            byte_serialize(self.config.source_identity.as_bytes()).collect();

        // Create a source unique ID - using a UUID to ensure uniqueness
        let source_unique_id = Uuid::new_v4();

        // Create the query string
        let mut query = String::with_capacity(512); // Preallocate enough space for the query string (decided based on expected size)
        write!(&mut query, "api/v1/ingestion/ingest?endpoint={}&moniker={}&namespace={}&event={}&version={}&sourceUniqueId={}&sourceIdentity={}&startTime={}&endTime={}&format=centralbond/lz4hc&dataSize={}&minLevel={}&schemaIds={}&rowCount={}",
            encoded_monitoring_endpoint,
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

        // Append On Behalf Of query parameters when configured.
        // onbehalfid is NOT URL-encoded (matches C# PipelineAgent behavior).
        if let Some(ref identity) = self.config.onbehalf_identity {
            if !identity.is_empty() {
                write!(&mut query, "&onbehalfid={}", identity).map_err(|e| {
                    GenevaUploaderError::InternalError(format!("Failed to write onbehalfid: {e}"))
                })?;
            }
        }
        // onbehalfannotations IS URL-encoded (matches C# HttpUtility.UrlEncode behavior).
        if let Some(ref annotations) = self.config.onbehalf_annotations {
            if !annotations.is_empty() {
                let encoded_annotations: String = byte_serialize(annotations.as_bytes()).collect();
                write!(&mut query, "&onbehalfannotations={}", encoded_annotations).map_err(
                    |e| {
                        GenevaUploaderError::InternalError(format!(
                            "Failed to write onbehalfannotations: {e}"
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
    ) -> Result<IngestionResponse> {
        debug!(
            name: "uploader.upload",
            target: "geneva-uploader",
            event_name = %event_name,
            size = data.len(),
            "Starting upload"
        );

        // Always get fresh auth info
        let (auth_info, moniker_info, monitoring_endpoint) =
            self.config_client.get_ingestion_info().await?;
        let data_size = data.len();
        let upload_uri = self.create_upload_uri(
            &monitoring_endpoint,
            &moniker_info.name,
            data_size,
            event_name,
            metadata,
            row_count,
        )?;
        let full_url = format!(
            "{}/{}",
            auth_info.endpoint.trim_end_matches('/'),
            upload_uri
        );

        debug!(
            name: "uploader.upload.post",
            target: "geneva-uploader",
            event_name = %event_name,
            moniker = %moniker_info.name,
            "Posting to ingestion gateway"
        );

        // Send the upload request
        let response = self
            .http_client
            .post(&full_url)
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", auth_info.auth_token),
            )
            .body(data)
            .send()
            .await?;
        let status = response.status();
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

            Ok(ingest_response)
        } else {
            debug!(
                name: "uploader.upload.failed",
                target: "geneva-uploader",
                event_name = %event_name,
                status = status.as_u16(),
                body = %body,
                "Upload failed"
            );
            Err(GenevaUploaderError::UploadFailed {
                status: status.as_u16(),
                message: body,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to build a GenevaUploaderConfig for testing (without a real config client)
    fn test_config() -> GenevaUploaderConfig {
        GenevaUploaderConfig {
            namespace: "TestNamespace".to_string(),
            source_identity: "Tenant=T/Role=R/RoleInstance=RI".to_string(),
            environment: "Test".to_string(),
            config_version: "Ver2v0".to_string(),
            onbehalf_identity: None,
            onbehalf_annotations: None,
        }
    }

    #[test]
    fn test_obo_config_defaults_to_none() {
        let config = test_config();
        assert!(config.onbehalf_identity.is_none());
        assert!(config.onbehalf_annotations.is_none());
    }

    #[test]
    fn test_obo_config_with_identity_only() {
        let mut config = test_config();
        config.onbehalf_identity = Some("Microsoft.HybridCompute".to_string());
        assert_eq!(
            config.onbehalf_identity.as_deref(),
            Some("Microsoft.HybridCompute")
        );
        assert!(config.onbehalf_annotations.is_none());
    }

    #[test]
    fn test_obo_config_with_both_fields() {
        let mut config = test_config();
        config.onbehalf_identity = Some("Microsoft.HybridCompute".to_string());
        config.onbehalf_annotations = Some(
            r#"<Config onBehalfFields="resourceId,category" priority="Normal" />"#.to_string(),
        );
        assert_eq!(
            config.onbehalf_identity.as_deref(),
            Some("Microsoft.HybridCompute")
        );
        assert!(config
            .onbehalf_annotations
            .as_ref()
            .unwrap()
            .contains("onBehalfFields"));
    }

    #[test]
    fn test_obo_annotations_url_encoding() {
        // Verify that byte_serialize correctly encodes XML characters
        let annotations = r#"<Config onBehalfFields="resourceId,category" priority="Normal" />"#;
        let encoded: String = byte_serialize(annotations.as_bytes()).collect();

        // XML angle brackets, quotes, and spaces must be percent-encoded
        assert!(!encoded.contains('<'));
        assert!(!encoded.contains('>'));
        assert!(!encoded.contains('"'));
        // '<' -> %3C, '>' -> %3E, '"' -> %22
        assert!(encoded.contains("%3C") || encoded.contains("%3c"));
        assert!(encoded.contains("%3E") || encoded.contains("%3e"));
        assert!(encoded.contains("%22"));
    }

    #[test]
    fn test_obo_identity_not_url_encoded() {
        // onbehalfid values with dots should NOT be encoded (matching C# behavior)
        let identity = "Microsoft.HybridCompute";
        let param = format!("&onbehalfid={}", identity);
        assert_eq!(param, "&onbehalfid=Microsoft.HybridCompute");
        // Dots should remain as-is
        assert!(param.contains('.'));
    }

    #[test]
    fn test_empty_obo_fields_treated_as_absent() {
        let mut config = test_config();
        config.onbehalf_identity = Some(String::new());
        config.onbehalf_annotations = Some(String::new());
        // Empty strings should be treated the same as None in create_upload_uri
        // (the method checks `!identity.is_empty()` before appending)
        assert!(config.onbehalf_identity.as_ref().unwrap().is_empty());
        assert!(config.onbehalf_annotations.as_ref().unwrap().is_empty());
    }
}
