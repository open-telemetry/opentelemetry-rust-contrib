//! High-level GenevaClient for user code. Wraps config_service and ingestion_service.

use crate::config_service::client::{AuthMethod, GenevaConfigClient, GenevaConfigClientConfig};
use crate::ingestion_service::uploader::{GenevaUploader, GenevaUploaderConfig};
use crate::payload_encoder::lz4_chunked_compression::lz4_chunked_compression;
use crate::payload_encoder::otlp_encoder::OtlpEncoder;
use opentelemetry_proto::tonic::logs::v1::ResourceLogs;
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
    // Add event name/version here if constant, or per-upload if you want them per call.
}

/// Main user-facing client for Geneva ingestion.
#[derive(Clone)]
pub struct GenevaClient {
    uploader: Arc<GenevaUploader>,
    encoder: Arc<OtlpEncoder>,
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

        let schema_ids =
            "c1ce0ecea020359624c493bbe97f9e80;0da22cabbee419e000541a5eda732eb3".to_string(); // TODO - find the actual value to be populated

        // Uploader config
        let uploader_config = GenevaUploaderConfig {
            namespace: cfg.namespace.clone(),
            source_identity,
            environment: cfg.environment,
            schema_ids,
        };

        let uploader = GenevaUploader::from_config_client(config_client, uploader_config)
            .await
            .map_err(|e| format!("GenevaUploader init failed: {e}"))?;
        let metadata = format!(
            "namespace={}/eventVersion={}/tenant={}/role={}/roleinstance={}",
            cfg.namespace,
            "Ver1v0", // You can replace this with a cfg field if version should be dynamic
            cfg.tenant,
            cfg.role_name,
            cfg.role_instance,
        );
        Ok(Self {
            uploader: Arc::new(uploader),
            encoder: Arc::new(OtlpEncoder::new()),
            metadata,
        })
    }

    /// Upload OTLP logs (as ResourceLogs).
    pub async fn upload_logs(&self, logs: Vec<ResourceLogs>) -> Result<(), String> {
        let log_iter = logs
            .iter()
            .flat_map(|resource_log| resource_log.scope_logs.iter())
            .flat_map(|scope_log| scope_log.log_records.iter());
        let blobs = self.encoder.encode_log_batch(log_iter, &self.metadata);
        for (_schema_id, event_name, encoded_blob, _row_count) in blobs {
            // TODO - log encoded_blob for debugging
            let compressed_blob = lz4_chunked_compression(&encoded_blob)
                .map_err(|e| format!("LZ4 compression failed: {e}"))?;
            // TODO - log compressed_blob for debugging
            let event_version = "Ver2v0"; // TODO - find the actual value to be populated
            println!("--->> Uploading event: {event_name}");
            self.uploader
                .upload(compressed_blob, &event_name, event_version)
                .await
                .map_err(|e| format!("Geneva upload failed: {e}"))?;
        }
        Ok(())
    }
}
