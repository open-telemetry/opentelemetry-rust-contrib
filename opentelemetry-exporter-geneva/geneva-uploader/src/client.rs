//! High-level GenevaClient for user code. Wraps config_service and ingestion_service.

use crate::config_service::client::{AuthMethod, GenevaConfigClient, GenevaConfigClientConfig};
use crate::ingestion_service::uploader::{GenevaUploader, GenevaUploaderConfig};
use crate::payload_encoder::encoder::Encoder;
use crate::payload_encoder::lz4_chunked_compression::lz4_chunked_compression;
use opentelemetry_proto::tonic::logs::v1::ResourceLogs;
use std::sync::Arc;

use std::fs::File;
use std::io::Write;

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
    encoder: Arc<Encoder>,
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
            encoder: Arc::new(Encoder::new()),
            metadata,
        })
    }

    /// Upload OTLP logs (as ResourceLogs).
    pub async fn upload_logs(&self, logs: Vec<ResourceLogs>) -> Result<(), String> {
        // For each log, convert to EncoderField(s) and upload via GenevaUploader
        for resource_log in logs {
            for scope_log in &resource_log.scope_logs {
                for log_record in &scope_log.log_records {
                    let event_name = &log_record.event_name;
                    let mut fields: [crate::payload_encoder::encoder::EncoderField; 10] =
                        std::array::from_fn(|_| {
                            crate::payload_encoder::encoder::EncoderField::default()
                        });
                    let n = crate::payload_encoder::otlp_mapper::log_record_to_encoder_fields(
                        log_record,
                        &mut fields,
                    );
                    let encoder_fields = &fields[..n];
                    let level = 1u8; //TODO - find the actual value to be populated (severity?)

                    // Encode using the encoder (could be part of uploader or a utility function)
                    let encoded_blob = self.encoder.encode(
                        encoder_fields,
                        event_name,
                        level,
                        self.metadata.as_str(),
                    );
                    println!("Encoded blob: {:?}", encoded_blob);
                    File::create("/tmp/final_uncompressed.blob")
                        .unwrap()
                        .write_all(&encoded_blob)
                        .unwrap();

                    let compressed_blob = lz4_chunked_compression(&encoded_blob)
                        .map_err(|e| format!("LZ4 compression failed: {e}"))?; //TODO - error handling
                    File::create("/tmp/final_compressed.blob")
                        .unwrap()
                        .write_all(&compressed_blob)
                        .unwrap();
                    // Upload using the internal uploader

                    let event_version = "Ver2v0"; // TODO - find the actual value to be populated
                    self.uploader
                        .upload(compressed_blob, event_name, event_version)
                        .await
                        .map_err(|e| format!("Geneva upload failed: {e}"))?;
                }
            }
        }
        Ok(())
    }
}
