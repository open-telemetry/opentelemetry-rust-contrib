//! High-level GenevaClient for user code. Wraps config_service and ingestion_service.

use crate::config_service::client::{AuthMethod, GenevaConfigClient, GenevaConfigClientConfig};
// ManagedIdentitySelector removed; no re-export needed.
use crate::ingestion_service::uploader::{GenevaUploader, GenevaUploaderConfig};
use crate::payload_encoder::otlp_encoder::MetadataFields;
use crate::payload_encoder::otlp_encoder::OtlpEncoder;
use opentelemetry_proto::tonic::logs::v1::ResourceLogs;
use opentelemetry_proto::tonic::trace::v1::ResourceSpans;
use std::sync::Arc;
use tracing::{debug, info};

/// Public batch type (already LZ4 chunked compressed).
/// Produced by `OtlpEncoder::encode_log_batch` and returned to callers.
#[derive(Debug, Clone)]
pub struct EncodedBatch {
    pub event_name: String,
    pub data: Vec<u8>,
    pub metadata: crate::payload_encoder::central_blob::BatchMetadata,
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

    /// Encode OTLP logs into LZ4 chunked compressed batches.
    pub fn encode_and_compress_logs(
        &self,
        logs: &[ResourceLogs],
    ) -> Result<Vec<EncodedBatch>, String> {
        debug!(
            name: "client.encode_and_compress_logs",
            target: "geneva-uploader",
            resource_logs_count = logs.len(),
            "Encoding and compressing resource logs"
        );

        let log_iter = logs
            .iter()
            .flat_map(|resource_log| resource_log.scope_logs.iter())
            .flat_map(|scope_log| scope_log.log_records.iter());

        self.encoder
            .encode_log_batch(log_iter, &self.metadata_fields)
            .map_err(|e| {
                debug!(
                    name: "client.encode_and_compress_logs.error",
                    target: "geneva-uploader",
                    error = %e,
                    "Log compression failed"
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
    pub async fn upload_batch(&self, batch: &EncodedBatch) -> Result<(), String> {
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
                format!("Geneva upload failed: {e} Event: {}", batch.event_name)
            })
    }
}
