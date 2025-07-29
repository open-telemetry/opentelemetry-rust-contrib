use core::fmt;
use geneva_uploader::client::GenevaClient;
use opentelemetry_proto::transform::common::tonic::ResourceAttributesWithSchema;
use opentelemetry_proto::transform::logs::tonic::group_logs_by_resource_and_scope;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::logs::LogBatch;
use std::sync::atomic;

/// An OpenTelemetry exporter that writes logs to Geneva exporter
pub struct GenevaExporter {
    resource: ResourceAttributesWithSchema,
    _is_shutdown: atomic::AtomicBool,
    geneva_client: GenevaClient,
}

impl GenevaExporter {
    /// Create a new GenavaExporter
    pub fn new(geneva_client: GenevaClient) -> Self {
        Self {
            resource: ResourceAttributesWithSchema::default(),
            _is_shutdown: atomic::AtomicBool::new(false),
            geneva_client,
        }
    }
}

impl fmt::Debug for GenevaExporter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Genava exporter")
    }
}

impl opentelemetry_sdk::logs::LogExporter for GenevaExporter {
    async fn export(&self, batch: LogBatch<'_>) -> OTelSdkResult {
        let otlp = group_logs_by_resource_and_scope(batch, &self.resource);

        // Encode and compress logs into batches
        let compressed_batches = match self.geneva_client.encode_and_compress_logs(&otlp) {
            Ok(batches) => batches,
            Err(e) => return Err(OTelSdkError::InternalFailure(e)),
        };

        // Upload each batch individually
        for batch in &compressed_batches {
            if let Err(e) = self.geneva_client.upload_batch(batch).await {
                return Err(OTelSdkError::InternalFailure(e));
            }
        }

        Ok(())
    }

    fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
        self.resource = resource.into();
    }
}
