use core::fmt;
use futures::stream::{self, StreamExt};
use geneva_uploader::client::GenevaClient;
use opentelemetry_proto::transform::common::tonic::ResourceAttributesWithSchema;
use opentelemetry_proto::transform::trace::tonic::group_spans_by_resource_and_scope;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::trace::SpanExporter;
use std::sync::{atomic, Arc};

/// An OpenTelemetry exporter that writes spans to Geneva exporter
pub struct GenevaTraceExporter {
    resource: ResourceAttributesWithSchema,
    _is_shutdown: atomic::AtomicBool,
    geneva_client: Arc<GenevaClient>,
    max_concurrent_uploads: usize,
}

// TODO - Add builder pattern for GenevaTraceExporter to allow more flexible configuration
impl GenevaTraceExporter {
    /// Create a new GenevaTraceExporter
    pub fn new(geneva_client: GenevaClient) -> Self {
        Self::new_with_concurrency(geneva_client, 4) // Default to 4 concurrent uploads
    }

    /// Create a new GenevaTraceExporter with custom concurrency level
    pub fn new_with_concurrency(
        geneva_client: GenevaClient,
        max_concurrent_uploads: usize,
    ) -> Self {
        Self {
            resource: ResourceAttributesWithSchema::default(),
            _is_shutdown: atomic::AtomicBool::new(false),
            geneva_client: Arc::new(geneva_client),
            max_concurrent_uploads,
        }
    }
}

impl fmt::Debug for GenevaTraceExporter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Geneva trace exporter")
    }
}

impl SpanExporter for GenevaTraceExporter {
    async fn export(&self, batch: Vec<opentelemetry_sdk::trace::SpanData>) -> OTelSdkResult {
        let otlp = group_spans_by_resource_and_scope(batch, &self.resource);

        // Encode and compress spans into batches
        let compressed_batches = match self.geneva_client.encode_and_compress_spans(&otlp) {
            Ok(batches) => batches,
            Err(e) => return Err(OTelSdkError::InternalFailure(e)),
        };

        // Execute uploads concurrently within the same async task using buffer_unordered.
        // This processes up to max_concurrent_uploads batches simultaneously without
        // spawning new tasks or threads, using async I/O concurrency instead.
        // All batch uploads are processed asynchronously in the same task context that
        // called the export() method.
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

    fn shutdown(&mut self) -> OTelSdkResult {
        // Set shutdown flag to true
        self._is_shutdown.store(true, atomic::Ordering::Relaxed);
        // TODO: Use the is_shutdown value in export() method to prevent exports after shutdown
        Ok(())
    }
}
