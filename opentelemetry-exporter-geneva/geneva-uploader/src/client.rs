//! High-level GenevaClient for user code. Wraps config_service and ingestion_service.

use crate::config_service::client::{AuthMethod, GenevaConfigClient, GenevaConfigClientConfig};
use crate::ingestion_service::uploader::{GenevaUploader, GenevaUploaderConfig};
use crate::payload_encoder::otlp_encoder::OtlpEncoder;
use opentelemetry_proto::tonic::logs::v1::ResourceLogs;
use std::sync::Arc;

/// Public batch type (already LZ4 chunked compressed).
/// Produced by `OtlpEncoder::encode_log_batch` and returned to callers.
#[derive(Debug, Clone)]
pub struct EncodedBatch {
    pub event_name: String,
    pub data: Vec<u8>,
    pub metadata: crate::payload_encoder::central_blob::BatchMetadata,
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
    // Add event name/version here if constant, or per-upload if you want them per call.
}

/// Main user-facing client for Geneva ingestion.
#[derive(Clone)]
pub struct GenevaClient {
    uploader: Arc<GenevaUploader>,
    encoder: OtlpEncoder,
    metadata: String,
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
        };

        let uploader = GenevaUploader::from_config_client(config_client, uploader_config)
            .await
            .map_err(|e| format!("GenevaUploader init failed: {e}"))?;
        Ok(Self {
            uploader: Arc::new(uploader),
            encoder: OtlpEncoder::new(),
            metadata,
        })
    }

    /// Encode OTLP logs into LZ4 chunked compressed batches.
    pub fn encode_and_compress_logs(
        &self,
        logs: &[ResourceLogs],
    ) -> Result<Vec<EncodedBatch>, String> {
        let log_iter = logs
            .iter()
            .flat_map(|resource_log| resource_log.scope_logs.iter())
            .flat_map(|scope_log| scope_log.log_records.iter());

        self.encoder
            .encode_log_batch(log_iter, &self.metadata)
            .map_err(|e| format!("Compression failed: {e}"))
    }

    /// Upload a single compressed batch.
    /// This allows for granular control over uploads, including custom retry logic for individual batches.
    pub async fn upload_batch(&self, batch: &EncodedBatch) -> Result<(), String> {
        self.uploader
            .upload(batch.data.clone(), &batch.event_name, &batch.metadata)
            .await
            .map(|_| ())
            .map_err(|e| format!("Geneva upload failed: {e} Event: {}", batch.event_name))
    }
}
