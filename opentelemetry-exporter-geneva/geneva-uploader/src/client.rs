//! High-level GenevaClient for user code. Wraps config_service and ingestion_service.

use crate::config_service::client::{AuthMethod, GenevaConfigClient, GenevaConfigClientConfig};
// ManagedIdentitySelector removed; no re-export needed.
use crate::ingestion_service::uploader::{
    GenevaUploader, GenevaUploaderConfig, GenevaUploaderError,
};
use crate::payload_encoder::otlp_encoder::MetadataFields;
use crate::payload_encoder::otlp_encoder::OtlpEncoder;
use opentelemetry_proto::tonic::trace::v1::ResourceSpans;
use otap_df_pdata_views::views::logs::LogsDataView;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info};

/// Public batch type (already LZ4 chunked compressed).
/// Produced by `OtlpEncoder::encode_log_batch` and returned to callers.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedBatch {
    pub event_name: String,
    pub(crate) data: Vec<u8>,
    pub(crate) metadata: crate::payload_encoder::central_blob::BatchMetadata,
    pub row_count: usize,
}

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
    pub msi_resource: Option<String>, // Required for Managed Identity variants
                                      // Add event name/version here if constant, or per-upload if you want them per call.
}

/// Error type returned by [`GenevaClient::upload_batch`].
///
/// Provides enough information for callers to implement retry strategies:
/// - [`HttpStatus`](UploadError::HttpStatus) carries the HTTP status code and
///   an optional `Retry-After` duration so callers can distinguish retriable
///   server errors (429, 5xx) from permanent client errors (4xx).
/// - [`Transport`](UploadError::Transport) indicates a network-level failure
///   (timeout, connection refused, DNS) that is typically retriable.
/// - [`Other`](UploadError::Other) covers config-service or internal errors.
#[derive(Debug)]
pub enum UploadError {
    /// Server returned a non-202 HTTP status.
    HttpStatus {
        status: u16,
        retry_after: Option<Duration>,
        message: String,
    },
    /// Network/transport failure (timeout, connection refused, DNS, etc.)
    Transport(String),
    /// Config service or other internal error.
    Other(String),
}

impl fmt::Display for UploadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HttpStatus {
                status, message, ..
            } => {
                write!(f, "upload failed with status {status}: {message}")
            }
            Self::Transport(msg) => write!(f, "transport error: {msg}"),
            Self::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for UploadError {}

/// Main user-facing client for Geneva ingestion.
#[derive(Clone)]
pub struct GenevaClient {
    uploader: Arc<GenevaUploader>,
    encoder: OtlpEncoder,
    metadata_fields: MetadataFields,
}

impl GenevaClient {
    pub fn new(cfg: GenevaClientConfig) -> Result<Self, String> {
        info!(
            name: "client.new",
            target: "geneva-uploader",
            endpoint = %cfg.endpoint,
            namespace = %cfg.namespace,
            account = %cfg.account,
            "Initializing GenevaClient"
        );

        // Validate MSI resource presence for managed identity variants
        match cfg.auth_method {
            AuthMethod::SystemManagedIdentity
            | AuthMethod::UserManagedIdentity { .. }
            | AuthMethod::UserManagedIdentityByObjectId { .. }
            | AuthMethod::UserManagedIdentityByResourceId { .. } => {
                if cfg.msi_resource.is_none() {
                    debug!(
                        name: "client.new.validate_msi_resource",
                        target: "geneva-uploader",
                        "Validation failed: msi_resource must be provided for managed identity auth"
                    );
                    return Err(
                        "msi_resource must be provided for managed identity auth".to_string()
                    );
                }
            }
            AuthMethod::Certificate { .. } => {}
            AuthMethod::WorkloadIdentity { .. } => {}
            #[cfg(feature = "mock_auth")]
            AuthMethod::MockAuth => {}
        }
        let config_client_config = GenevaConfigClientConfig {
            endpoint: cfg.endpoint,
            environment: cfg.environment.clone(),
            account: cfg.account,
            namespace: cfg.namespace.clone(),
            region: cfg.region,
            config_major_version: cfg.config_major_version,
            auth_method: cfg.auth_method,
            msi_resource: cfg.msi_resource,
            #[cfg(test)]
            test_root_ca_pem: None,
        };
        let config_client =
            Arc::new(GenevaConfigClient::new(config_client_config).map_err(|e| {
                debug!(
                    name: "client.new.config_client_init",
                    target: "geneva-uploader",
                    error = %e,
                    "GenevaConfigClient init failed"
                );
                format!("GenevaConfigClient init failed: {e}")
            })?);

        let source_identity = format!(
            "Tenant={}/Role={}/RoleInstance={}",
            cfg.tenant, cfg.role_name, cfg.role_instance
        );

        let config_version = format!("Ver{}v0", cfg.config_major_version);

        // Create metadata fields that will appear as Bond schema fields in Geneva
        let metadata_fields = MetadataFields::new(
            cfg.environment,
            config_version.clone(),
            cfg.tenant,
            cfg.role_name,
            cfg.role_instance,
            cfg.namespace,
            config_version,
        );

        let uploader_config = GenevaUploaderConfig {
            namespace: metadata_fields.namespace.clone(),
            source_identity,
            environment: metadata_fields.env_name.clone(),
            config_version: metadata_fields.event_version.clone(),
        };

        let uploader =
            GenevaUploader::from_config_client(config_client, uploader_config).map_err(|e| {
                debug!(
                    name: "client.new.uploader_init",
                    target: "geneva-uploader",
                    error = %e,
                    "GenevaUploader init failed"
                );
                format!("GenevaUploader init failed: {e}")
            })?;

        info!(
            name: "client.new.complete",
            target: "geneva-uploader",
            "GenevaClient initialized successfully"
        );

        Ok(Self {
            uploader: Arc::new(uploader),
            encoder: OtlpEncoder::new(),
            metadata_fields,
        })
    }

    /// Encode logs from any [`LogsDataView`] implementation into LZ4-chunked
    /// compressed batches, grouped by event name.
    ///
    /// # What to implement
    ///
    /// Implement the following traits from `otap_df_pdata_views`:
    ///
    /// ```text
    /// LogsDataView
    /// └─ ResourceLogsView
    ///    └─ ScopeLogsView
    ///       └─ LogRecordView   ← one impl per log record type
    ///          └─ AnyValueView  (for body / attributes)
    ///          └─ AttributeView (for attributes)
    /// ```
    ///
    /// The `event_name` field on each log record controls which Geneva event
    /// table the record is routed to.  Records with no event name (or an
    /// empty one) are routed to the `"Log"` table.
    ///
    /// # Usage pattern
    ///
    /// ```ignore
    /// let batches = client.encode_and_compress_logs(&my_view)?;
    /// for batch in &batches {
    ///     client.upload_batch(batch).await?;
    /// }
    /// ```
    ///
    /// See `examples/view_basic.rs` for the common `RawLogsData` usage pattern
    /// and `examples/view_advanced.rs` for a full custom `LogsDataView`
    /// implementation.
    pub fn encode_and_compress_logs<T: LogsDataView>(
        &self,
        view: &T,
    ) -> Result<Vec<EncodedBatch>, String> {
        debug!(
            name: "client.encode_and_compress_logs",
            target: "geneva-uploader",
            "Encoding and compressing logs"
        );

        self.encoder
            .encode_logs_from_view(view, &self.metadata_fields)
            .map_err(|e| {
                debug!(
                    name: "client.encode_and_compress_logs.error",
                    target: "geneva-uploader",
                    error = %e,
                    "Logs compression failed"
                );
                format!("Compression failed: {e}")
            })
    }

    /// Encode OTLP spans into LZ4 chunked compressed batches.
    pub fn encode_and_compress_spans(
        &self,
        spans: &[ResourceSpans],
    ) -> Result<Vec<EncodedBatch>, String> {
        debug!(
            name: "client.encode_and_compress_spans",
            target: "geneva-uploader",
            resource_spans_count = spans.len(),
            "Encoding and compressing resource spans"
        );

        let span_iter = spans
            .iter()
            .flat_map(|resource_span| resource_span.scope_spans.iter())
            .flat_map(|scope_span| scope_span.spans.iter());

        self.encoder
            .encode_span_batch(span_iter, &self.metadata_fields)
            .map_err(|e| {
                debug!(
                    name: "client.encode_and_compress_spans.error",
                    target: "geneva-uploader",
                    error = %e,
                    "Span compression failed"
                );
                format!("Compression failed: {e}")
            })
    }

    /// Upload a single compressed batch.
    /// This allows for granular control over uploads, including custom retry logic for individual batches.
    pub async fn upload_batch(&self, batch: &EncodedBatch) -> Result<(), UploadError> {
        debug!(
            name: "client.upload_batch",
            target: "geneva-uploader",
            event_name = %batch.event_name,
            size = batch.data.len(),
            "Uploading batch"
        );

        self.uploader
            .upload(
                batch.data.clone(),
                &batch.event_name,
                &batch.metadata,
                batch.row_count,
            )
            .await
            .map(|_| {
                debug!(
                    name: "client.upload_batch.success",
                    target: "geneva-uploader",
                    event_name = %batch.event_name,
                    "Successfully uploaded batch"
                );
            })
            .map_err(|e| {
                debug!(
                    name: "client.upload_batch.error",
                    target: "geneva-uploader",
                    event_name = %batch.event_name,
                    error = %e,
                    "Geneva upload failed"
                );
                match e {
                    GenevaUploaderError::UploadFailed {
                        status,
                        retry_after,
                        message,
                    } => UploadError::HttpStatus {
                        status,
                        retry_after,
                        message,
                    },
                    GenevaUploaderError::Http(msg) => UploadError::Transport(msg),
                    other => UploadError::Other(other.to_string()),
                }
            })
    }
}
