use chrono::{DateTime, Duration as ChronoDuration, Utc};

use crate::config_service::client::{GenevaConfigClient, GenevaConfigClientError};
use chrono::{Datelike, Timelike};
use reqwest::{header, Client};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use url::form_urlencoded::byte_serialize;
use uuid::Uuid;

/// Error types for the Geneva Uploader
#[derive(Debug, Error)]
pub(crate) enum GenevaUploaderError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
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
        GenevaUploaderError::ConfigClient(format!("GenevaConfigClient error: {}", err))
    }
}

pub(crate) type Result<T> = std::result::Result<T, GenevaUploaderError>;

#[allow(dead_code)]
/// Response from the ingestion API when submitting data
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct IngestionResponse {
    pub(crate) ticket: String,
    #[serde(flatten)]
    pub(crate) extra: HashMap<String, Value>,
}

/// Configuration for the Geneva Uploader
#[derive(Debug, Clone)]
pub(crate) struct GenevaUploaderConfig {
    pub namespace: String,
    pub source_identity: String,
    #[allow(dead_code)]
    pub environment: String,
    pub schema_ids: String,
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
    pub(crate) async fn from_config_client(
        config_client: Arc<GenevaConfigClient>,
        uploader_config: GenevaUploaderConfig,
    ) -> Result<Self> {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            header::HeaderValue::from_static("application/json"),
        );
        let http_client = Client::builder()
            .timeout(Duration::from_secs(30))
            .default_headers(headers)
            .build()?;

        Ok(Self {
            config_client,
            config: uploader_config,
            http_client,
        })
    }

    /// Creates the GIG upload URI with required parameters
    #[allow(dead_code)]
    fn create_upload_uri(
        &self,
        monitoring_endpoint: &str,
        moniker: &str,
        data_size: usize,
        event_name: &str,
        event_version: &str,
    ) -> Result<String> {
        let now: DateTime<Utc> = Utc::now(); //TODO - this need to be calculated from the bond data
        let end_time = now + ChronoDuration::minutes(5); //TODO - this need to be calculated from the bond data

        // Format times in ISO 8601 format with fixed precision
        // Using .NET compatible format (matches DateTime.ToString("O"))

        let start_time = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:07}Z",
            now.year(),
            now.month(),
            now.day(),
            now.hour(),
            now.minute(),
            now.second(),
            now.nanosecond() / 100 // Convert nanoseconds to 7-digit precision
        );

        let end_time = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:07}Z",
            end_time.year(),
            end_time.month(),
            end_time.day(),
            end_time.hour(),
            end_time.minute(),
            end_time.second(),
            end_time.nanosecond() / 100 // Convert nanoseconds to 7-digit precision
        );

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
        write!(&mut query, "api/v1/ingestion/ingest?endpoint={}&moniker={}&namespace={}&event={}&version={}&sourceUniqueId={}&sourceIdentity={}&startTime={}&endTime={}&format=centralbond/lz4hc&dataSize={}&minLevel={}&schemaIds={}",
            encoded_monitoring_endpoint,
            moniker,
            self.config.namespace,
            event_name,
            event_version,
            source_unique_id,
            encoded_source_identity,
            start_time,
            end_time,
            data_size,
            2,
            self.config.schema_ids
        ).map_err(|e| GenevaUploaderError::InternalError(format!("Failed to write query string: {e}")))?;
        Ok(query)
    }

    /// Uploads data to the ingestion gateway
    ///
    /// # Arguments
    /// * `data` - The encoded data to upload (already in the required format)
    ///
    /// # Returns
    /// * `Result<IngestionResponse>` - The response containing the ticket ID or an error
    #[allow(dead_code)]
    pub(crate) async fn upload(
        &self,
        data: Vec<u8>,
        event_name: &str,
        event_version: &str,
    ) -> Result<IngestionResponse> {
        // Always get fresh auth info
        let (auth_info, moniker_info, monitoring_endpoint) =
            self.config_client.get_ingestion_info().await?;
        let data_size = data.len();
        let upload_uri = self.create_upload_uri(
            &monitoring_endpoint,
            &moniker_info.name,
            data_size,
            event_name,
            event_version,
        )?;
        let full_url = format!(
            "{}/{}",
            auth_info.endpoint.trim_end_matches('/'),
            upload_uri
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
            .await
            .map_err(GenevaUploaderError::Http)?;

        let status = response.status();
        let body = response.text().await.map_err(GenevaUploaderError::Http)?;

        if status == reqwest::StatusCode::ACCEPTED {
            let ingest_response: IngestionResponse =
                serde_json::from_str(&body).map_err(GenevaUploaderError::SerdeJson)?;
            Ok(ingest_response)
        } else {
            Err(GenevaUploaderError::UploadFailed {
                status: status.as_u16(),
                message: body,
            })
        }
    }
}
