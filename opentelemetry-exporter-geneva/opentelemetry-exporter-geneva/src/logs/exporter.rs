use core::fmt;
use futures::stream::{self, StreamExt};
use geneva_uploader::client::GenevaClient;
use opentelemetry_proto::transform::common::tonic::ResourceAttributesWithSchema;
use opentelemetry_proto::transform::logs::tonic::group_logs_by_resource_and_scope;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::logs::LogBatch;
use std::sync::{atomic, Arc};

/// An OpenTelemetry exporter that writes logs to Geneva exporter
pub struct GenevaExporter {
    resource: ResourceAttributesWithSchema,
    _is_shutdown: atomic::AtomicBool,
    geneva_client: Arc<GenevaClient>,
    max_concurrent_uploads: usize,
}

impl GenevaExporter {
    /// Create a new GenavaExporter
    pub fn new(geneva_client: GenevaClient) -> Self {
        Self::with_concurrency(geneva_client, 10) // Default to 10 concurrent uploads
    }

    /// Create a new GenavaExporter with custom concurrency level
    pub fn with_concurrency(geneva_client: GenevaClient, max_concurrent_uploads: usize) -> Self {
        Self {
            resource: ResourceAttributesWithSchema::default(),
            _is_shutdown: atomic::AtomicBool::new(false),
            geneva_client: Arc::new(geneva_client),
            max_concurrent_uploads,
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

        // Execute uploads concurrently with configurable concurrency using buffer_unordered
        let errors: Vec<String> = stream::iter(compressed_batches)
            .map(|batch| {
                let client = self.geneva_client.clone();
                async move { client.upload_batch(&batch).await }
            })
            .buffer_unordered(self.max_concurrent_uploads)
            .filter_map(|result| async move { result.err() })
            .collect()
            .await;

        // Return error if any uploads failed
        if !errors.is_empty() {
            return Err(OTelSdkError::InternalFailure(format!(
                "Upload failures: {}",
                errors.join("; ")
            )));
        }

        Ok(())
    }

    fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
        self.resource = resource.into();
    }
}
