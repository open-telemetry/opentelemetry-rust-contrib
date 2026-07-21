//! High-level GenevaClient for user code. Wraps config_service and ingestion_service.

use crate::config_service::client::{AuthMethod, GenevaConfigClient, GenevaConfigClientConfig};
// ManagedIdentitySelector removed; no re-export needed.
use crate::ingestion_service::uploader::{
    GenevaUploader, GenevaUploaderConfig, GenevaUploaderError,
};
use crate::payload_encoder::otlp_encoder::OtlpEncoder;
use crate::payload_encoder::otlp_encoder::{lookup_obo_config, MetadataFields};
pub use crate::payload_encoder::otlp_encoder::{OboEventConfig, OboEventMap};
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

impl EncodedBatch {
    /// Returns the size in bytes of the compressed payload uploaded to Geneva.
    #[must_use]
    pub fn compressed_size(&self) -> usize {
        self.data.len()
    }
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
    pub logs: LogsConfig,
    pub spans: TracesConfig,
    pub obo_event_map: Option<OboEventMap>, // Per-event OBO config (None = no OBO)
}

#[derive(Clone, Debug)]
pub struct LogsConfig {
    pub default_event_name: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TracesConfig {
    pub default_event_name: Option<String>,
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
    log_table_name: Arc<str>,
    span_table_name: Arc<str>,
    obo_event_map: Option<OboEventMap>,
}

impl GenevaClient {
    pub fn new(cfg: GenevaClientConfig) -> Result<Self, String> {
        let log_table_name: Arc<str> = cfg
            .logs
            .default_event_name
            .as_deref()
            .unwrap_or("Log")
            .into();
        let span_table_name: Arc<str> = cfg
            .spans
            .default_event_name
            .as_deref()
            .unwrap_or("Span")
            .into();

        info!(
            name: "client.new",
            target: "geneva-uploader",
            endpoint = %cfg.endpoint,
            namespace = %cfg.namespace,
            account = %cfg.account,
            "Initializing GenevaClient"
        );

        info!(
            name: "client.new.geneva_event_name",
            target: "geneva-uploader",
            logs_default_event_name = %cfg.logs.default_event_name.as_deref().unwrap_or("<none>"),
            spans_default_event_name = %cfg.spans.default_event_name.as_deref().unwrap_or("<none>"),
            "Using LogsConfig and TracesConfig configuration"
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
            log_table_name,
            span_table_name,
            obo_event_map: cfg.obo_event_map,
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
            .encode_logs_from_view(
                view,
                &self.metadata_fields,
                self.log_table_name.as_ref(),
                self.obo_event_map.as_ref(),
            )
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
            .encode_span_batch(
                span_iter,
                &self.metadata_fields,
                self.span_table_name.as_ref(),
                self.obo_event_map.as_ref(),
            )
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

        // Look up per-event OBO config for this batch's event name
        let obo_config = lookup_obo_config(self.obo_event_map.as_ref(), &batch.event_name)
            .filter(|c| c.is_active());

        self.uploader
            .upload(
                batch.data.clone(),
                &batch.event_name,
                &batch.metadata,
                batch.row_count,
                obo_config,
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

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
    use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
    use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
    use otap_df_pdata::views::otlp::bytes::logs::RawLogsData;
    use prost::Message as _;

    fn build_config(logs: Option<&str>, spans: Option<&str>) -> GenevaClientConfig {
        GenevaClientConfig {
            endpoint: "https://example.test".to_string(),
            environment: "Test".to_string(),
            account: "acct".to_string(),
            namespace: "ns".to_string(),
            region: "eastus".to_string(),
            config_major_version: 2,
            auth_method: AuthMethod::WorkloadIdentity {
                resource: "https://monitor.azure.com".to_string(),
            },
            tenant: "tenant".to_string(),
            role_name: "role".to_string(),
            role_instance: "instance".to_string(),
            msi_resource: None,
            logs: LogsConfig {
                default_event_name: logs.map(str::to_owned),
            },
            spans: TracesConfig {
                default_event_name: spans.map(str::to_owned),
            },
            obo_event_map: None,
        }
    }

    fn build_client(logs: Option<&str>, spans: Option<&str>) -> GenevaClient {
        GenevaClient::new(build_config(logs, spans)).expect("client should initialize")
    }

    #[test]
    fn default_event_name_unwrap_or_prefers_override_and_falls_back() {
        let configured = maybe_event_name(true);
        let missing = maybe_event_name(false);

        assert_eq!(configured.unwrap_or("Log"), "AppLog");
        assert_eq!(missing.unwrap_or("Log"), "Log");
    }

    fn maybe_event_name(configured: bool) -> Option<&'static str> {
        if configured {
            Some("AppLog")
        } else {
            None
        }
    }

    #[test]
    fn encode_and_compress_logs_uses_configured_default_event_name() {
        let client = build_client(Some("AppLog"), None);

        let request = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                scope_logs: vec![ScopeLogs {
                    log_records: vec![LogRecord::default()],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };

        let bytes = request.encode_to_vec();
        let view = RawLogsData::new(&bytes);
        let batches = client
            .encode_and_compress_logs(&view)
            .expect("log encoding should succeed");

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].event_name, "AppLog");
    }

    #[test]
    fn encode_and_compress_spans_uses_configured_default_event_name() {
        let client = build_client(None, Some("AppTrace"));

        let spans = vec![ResourceSpans {
            scope_spans: vec![ScopeSpans {
                spans: vec![Span {
                    name: "span-name".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }];

        let batches = client
            .encode_and_compress_spans(&spans)
            .expect("span encoding should succeed");

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].event_name, "AppTrace");
    }
}
